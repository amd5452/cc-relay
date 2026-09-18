//! 基于订阅配额的本地透明接力（纯内存选路）。
//!
//! 开启本地路由后各智能体的 base_url 已指向本代理，运行期只需决定这一笔请求
//! 发往哪个上游——全部在内存完成，绝不回写任何智能体的配置文件。判定发生在
//! 请求发出之前，一次请求只打一个上游，因此不存在「输出到一半换 API」。
//!
//! 判定顺序：**未配置周期额度的供应商不参与接力**（方案 B——若把「没填配额」当成
//! 「不限量」，接力池会被无限撑大，按量付费兜底永远轮不到、429 也永远不可达）；
//! 标记按量付费 → 兜底档；无起点或窗口已过 → 记为「待重置」，进首选档；额度有
//! 剩余 → 首选档；否则跳过。首选档为空则用兜底档，两档皆空返回 429
//! （`ProxyError::QuotaExhausted`）。
//!
//! 选路分两阶段：先只做判定（纯读），确定最终链路之后才为**链路首个**供应商
//! 写入周期起点（首单计时）。这样备用供应商不会被提前开窗、白白损失周期时长。
//!
//! # 怎么用日志排查「接力没生效」
//!
//! 每次判定都会输出**一行** `配额接力` 汇总日志，列出每个候选供应商的处置结果
//! 与已用额度（默认 Info 级别即可见）。按下面的顺序对照即可定位：
//!
//! - **完全没有这一行** → 该请求没经过本地代理，或该应用没开启本地路由。
//! - **出现 `未启用配额接力`** → 没有任何供应商同时填了「周期上限」和「周期时长」。
//! - **出现 `配额配置不完整`** → 只填了上限或只填了时长，该项被按「不限量」处理
//!   （两项必须同时为正才会生效）。
//! - **某供应商显示 `已用 0/上限` 且从不上涨** → 请求没有成功，用量没落库；
//!   额度消耗只统计成功的请求，上游报错（key 无效、模型名不对）恒为 0。
//! - **显示 `周期起点在未来`** → 系统时钟回拨或数据库里是脏数据，此时
//!   `created_at >= 起点` 恒不成立，该供应商会一直「额度充足」。
//! - **出现 `需要重置周期`** → 该供应商窗口已过期或从未开始，被选中时会重置。
//! - **出现 `配额接力改变了选路`** → 接力确实发生了，可据此确认生效。
//! - **出现 `未配置模型路由映射`**（仅 Claude Desktop）→ 该供应商缺少模型路由
//!   映射，无法承接 Desktop 请求，已被接力排除。
//!
//! 应用差异：Claude Desktop 沿用**它自己那套**本地路由（独立 gateway、独立供应商
//! 命名空间），接力不需要任何额外接入——它的请求同样经过 `RequestContext::new`，
//! 因此天然复用本模块。唯一的差别是它要求供应商配好模型路由映射，见
//! [`provider_can_serve_app`]。

use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::provider_router::provider_supports_failover;
use crate::proxy::ProxyError;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// 小时 → 秒。
const SECONDS_PER_HOUR: f64 = 3600.0;

/// 周期起点晚于当前时间超过这个幅度，视为异常（时钟回拨或脏数据）。
const FUTURE_START_TOLERANCE_SECS: i64 = 300;

/// 已告警过的「配置问题」指纹。
///
/// 代理是热路径：如果某个供应商的配额度残缺，每个请求都会命中同一问题。
/// 按指纹去重，保证同一个问题只刷一次，既不丢诊断信息也不淹没日志。
fn warned_fingerprints() -> &'static Mutex<HashSet<String>> {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WARNED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 该指纹是否应输出告警（每个指纹只输出一次）。
///
/// 锁中毒时返回 `true`：宁可多打日志，也不要静默丢掉诊断信息。
fn should_warn(fingerprint: String) -> bool {
    warned_fingerprints()
        .lock()
        .map(|mut warned| warned.insert(fingerprint))
        .unwrap_or(true)
}

/// 该供应商是否声明了客户端请求的这条模型路由。
///
/// Claude Desktop 的每个供应商各自声明一份 route 表，**不同供应商的 route 集合
/// 可能不同**（例如某家的中转只映射了 sonnet/opus，没有 haiku）。客户端还会在
/// route 上带 `[1m]` 这类上下文标记，比较时需先剥掉。
fn provider_declares_route(provider: &Provider, requested_model: &str) -> bool {
    let requested = requested_model.trim();
    if requested.is_empty() {
        return true;
    }
    let lowered = requested.to_ascii_lowercase();
    let marker = crate::claude_desktop_config::ONE_M_CONTEXT_MARKER.to_ascii_lowercase();
    let base = lowered
        .strip_suffix(&marker)
        .map(str::trim_end)
        .unwrap_or(&lowered);

    crate::claude_desktop_config::proxy_model_routes(provider)
        .map(|routes| {
            routes.iter().any(|route| {
                let id = route.route_id.trim().to_ascii_lowercase();
                id == base || id == lowered
            })
        })
        .unwrap_or(false)
}

/// 该供应商能否承接指定应用的这次请求。
///
/// Claude Desktop 沿用**它自己那套**本地路由（独立 gateway + 独立供应商命名空间），
/// 且有两个硬前置条件：
/// 1. 必须配了「模型路由映射」——没配的转发时会抛 `InvalidRequest`；
/// 2. 必须声明了**本次请求要用的那条 route**——否则同样抛
///    `InvalidRequest`（"模型路由未配置"），而该错误不可重试，请求硬失败。
///
/// 这两点都在选路阶段排除，否则接力会把请求切到一个必然失败的候选上。
/// 其它应用没有这两个前置条件，一律放行。
fn provider_can_serve_app(
    app_type: &str,
    provider: &Provider,
    requested_model: Option<&str>,
) -> bool {
    if app_type != "claude-desktop" {
        return true;
    }
    let Ok(routes) = crate::claude_desktop_config::proxy_model_routes(provider) else {
        return false;
    };
    if routes.is_empty() {
        return false;
    }
    match requested_model {
        Some(model) => provider_declares_route(provider, model),
        // 拿不到请求模型时不做这层筛选，避免误杀。
        None => true,
    }
}

pub struct QuotaRelay;

impl QuotaRelay {
    /// 按配额对供应商重排序，返回实际使用的链路（首个元素即本次请求目标）。
    ///
    /// `preferred_id` 是配置里当前选中的供应商，会被优先尝试。
    /// 返回 `Ok(None)` 表示没有任何供应商参与配额接力，调用方应保留原有链路，
    /// 行为与改造前完全一致。
    ///
    /// # Errors
    /// 首选档与兜底档都为空时返回 [`ProxyError::QuotaExhausted`]。
    pub fn relay(
        db: &Database,
        app_type: &str,
        preferred_id: Option<&str>,
        requested_model: Option<&str>,
    ) -> Result<Option<Vec<Provider>>, ProxyError> {
        let all = db
            .get_all_providers(app_type)
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
        // 参与度判定。刻意区分「配置完整」与「配置残缺」：
        // 只填上限或只填时长是最常见的用户失误，后果却是「静默不生效」。
        let mut involved = false;
        for provider in all.values() {
            let max_tokens = provider.max_tokens_cycle.filter(|v| *v > 0);
            let duration_hours = provider.cycle_duration_hours.filter(|v| *v > 0.0);
            match (max_tokens, duration_hours) {
                (Some(_), Some(_)) => involved = true,
                (None, None) => involved |= provider.pay_as_you_go,
                _ => {
                    let fingerprint = format!(
                        "incomplete:{app_type}:{}:{:?}:{:?}",
                        provider.id, provider.max_tokens_cycle, provider.cycle_duration_hours
                    );
                    if should_warn(fingerprint) {
                        log::warn!("[{app_type}] 供应商 {} 配额配置不完整（上限={:?} / 时长={:?}），不参与接力；两项须同时填写才生效", provider.name, provider.max_tokens_cycle, provider.cycle_duration_hours);
                    }
                }
            }
        }
        let not_involved = !involved;
        if not_involved && should_warn(format!("no-participation:{app_type}")) {
            log::info!("[{app_type}] 未启用配额接力：没有任何供应商同时配置了周期上限与周期时长，也没有标记按量付费");
        }
        if not_involved {
            return Ok(None);
        }

        let now = chrono::Utc::now().timestamp();
        let mut ordered: Vec<&Provider> = Vec::with_capacity(all.len());
        if let Some(id) = preferred_id {
            if let Some(current) = all.get(id) {
                ordered.push(current);
            }
        }
        for (id, p) in &all {
            if Some(id.as_str()) != preferred_id {
                ordered.push(p);
            }
        }

        // 首选档记录「该供应商是否需要重置周期」，但此刻一律不写库。
        // 周期起点只能在确认它真会被使用时写入，否则会给尚未启用的备用供应商
        // 提前开窗，白白吃掉它的周期时长。
        let mut available: Vec<(&Provider, bool)> = Vec::new();
        let mut fallback: Vec<Provider> = Vec::new();
        // 仅因「该应用的前置条件不满足」而被排除的候选数量。用于区分
        // 「额度耗尽」与「压根没有可用候选」这两种空链路情形，避免误报 429。
        let mut precondition_excluded = 0usize;

        for p in ordered {
            // Codex 官方登录账号不能被借道转发到别的账号，排除。
            if !provider_supports_failover(app_type, p) {
                continue;
            }
            // 该应用特有的前置条件：Claude Desktop 既要配路由表，也要声明本次请求
            // 用的那条 route，缺任一条转发都会抛不可重试的 InvalidRequest。
            if !provider_can_serve_app(app_type, p, requested_model) {
                precondition_excluded += 1;
                if should_warn(format!("no-route:{app_type}:{}:{requested_model:?}", p.id)) {
                    log::info!(
                        "[{app_type}] 供应商 {} 无法承接本次请求（缺模型路由映射，或未声明 route {requested_model:?}），接力时跳过",
                        p.name
                    );
                }
                continue;
            }
            if p.pay_as_you_go {
                fallback.push(p.clone());
                continue;
            }
            let Some((max_tokens, hours)) = p.cycle_limit() else {
                // 未配置周期额度 → 不参与接力池。
                //
                // 这是刻意的（方案 B）：若把「没填配额」当成「不限量」，池子会被
                // 无限撑大——按量付费兜底永远轮不到、429 也永远不可达。
                if should_warn(format!("not-in-pool:{app_type}:{}", p.id)) {
                    log::info!(
                        "[{app_type}] 供应商 {} 未配置周期额度，不参与接力（需要无条件兜底请标记「按量付费」）",
                        p.name
                    );
                }
                continue;
            };
            let start = p.cycle_start_timestamp.unwrap_or(0);
            if start > now + FUTURE_START_TOLERANCE_SECS
                && should_warn(format!("future-start:{app_type}:{}:{start}", p.id))
            {
                log::warn!("[{app_type}] 供应商 {} 的周期起点（{start}）晚于当前时间，疑似时钟回拨或脏数据；该供应商会一直显示额度充足", p.name);
            }
            let deadline = start + (hours * SECONDS_PER_HOUR) as i64;
            if start <= 0 || deadline <= now {
                // 无起点或窗口已过期 → 需要重置。此处只登记，等确定被选中再写库。
                log::info!(
                    "[{app_type}] 供应商 {} 需要重置周期（原起点={start}、时长={hours}h、当前={now}）",
                    p.name
                );
                available.push((p, true));
                continue;
            }
            let used = db
                .sum_provider_tokens_since(&p.id, app_type, start)
                .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
            if used < max_tokens {
                log::info!(
                    "[{app_type}] 供应商 {} 本周期已用 {used}/{max_tokens}，额度充足，继续使用",
                    p.name
                );
                available.push((p, false));
            } else {
                log::info!(
                    "[{app_type}] 供应商 {} 本周期额度已满（{used}/{max_tokens}），尝试下一个",
                    p.name
                );
            }
        }

        let mut chain: Vec<Provider> = available.iter().map(|(p, _)| (*p).clone()).collect();
        chain.extend(fallback);

        if chain.is_empty() {
            // 池内无可服务者 → 明确失败（429），**不要**静默回退到原链路。
            //
            // 早期实现在「有候选因前置条件被排除」时返回 Ok(None) 回退原链路，
            // 本意是避免把非配额问题误报成 429；但实测后果是：请求继续打在已
            // 超限的供应商上，额度一路超支（实测到 516%）。静默超支比明确失败
            // 危险得多，因此这里一律报错，并说明真实原因。
            if precondition_excluded > 0 {
                log::warn!(
                    "[{app_type}] 接力池内无可用供应商：另有 {precondition_excluded} 个候选无法承接本次请求（缺模型路由映射，或未声明 route {requested_model:?}）"
                );
            }
            log::warn!("[{app_type}] 本周期额度全部耗尽，且没有可用的按量付费兜底");
            return Err(ProxyError::QuotaExhausted);
        }

        // 首单计时：只给最终会被使用的那个（链路首个）写周期起点。
        // 备用供应商留待真正被选中时再重置，避免提前消耗它的窗口时长。
        if let Some((winner, needs_reset)) = available.first() {
            if *needs_reset {
                Self::start_new_cycle(db, app_type, winner, now)?;
            }
        }

        let switched = preferred_id.is_some_and(|id| chain.first().is_some_and(|p| p.id != id));
        if switched {
            log::info!(
                "[{app_type}] 配额接力改变了选路：原首选 {preferred_id:?} → 实际使用「{}」",
                chain.first().map(|p| p.name.as_str()).unwrap_or("-")
            );
        }

        Ok(Some(chain))
    }

    /// 开启新周期：在发起请求前把周期起点写回数据库。
    fn start_new_cycle(
        db: &Database,
        app_type: &str,
        provider: &Provider,
        now: i64,
    ) -> Result<(), ProxyError> {
        db.set_provider_cycle_start(&provider.id, app_type, now)
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
        log::info!("[{app_type}] 供应商 {} 开始新的配额周期", provider.name);
        Ok(())
    }
}
