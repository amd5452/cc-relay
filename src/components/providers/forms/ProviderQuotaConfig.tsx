import { useTranslation } from "react-i18next";
import { Gauge } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

/** 订阅配额接力配置（对应 providers 表的真实列）。 */
export interface ProviderQuotaConfigValue {
  /** 原始 token 数（字符串形式）。界面以 M 为单位展示，本字段始终是原始值。 */
  maxTokensCycle?: string;
  cycleDurationHours?: string;
  payAsYouGo: boolean;
}

/**
 * 界面上「周期 Token 上限」以 **M（百万 token）** 为单位，数据库里仍存原始 token 数。
 *
 * 换算**只在本组件内部发生**，对外的 `quota.maxTokensCycle` 始终是原始值——
 * 这样后端与各表单的状态语义都不用改，也不存在「某个表单忘了换算」导致同一字段
 * 在不同表单里含义不同的风险。
 */
const TOKENS_PER_MILLION = 1_000_000;

/** 原始 token 数 → 界面显示的 M 数值。 */
function rawTokensToM(raw: string | undefined): string {
  if (!raw) return "";
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) return "";
  // toFixed 去掉浮点尾巴（如 1.2345670000000001），再 Number() 去掉多余的零
  return String(Number((parsed / TOKENS_PER_MILLION).toFixed(6)));
}

/**
 * 界面输入的 M 数值 → 原始 token 数。
 *
 * 必须 `Math.round`：`1.234567 * 1e6` 在 IEEE754 下可能得到 1234566.9999999998，
 * 而额度比较是 `已用 < 上限`，差 1 就足以让边界行为错乱。
 */
function mToRawTokens(millions: string): string | undefined {
  const trimmed = millions.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed <= 0) return undefined;
  return String(Math.round(parsed * TOKENS_PER_MILLION));
}

interface ProviderQuotaConfigProps {
  quota: ProviderQuotaConfigValue;
  onChange: (quota: ProviderQuotaConfigValue) => void;
}

/**
 * 订阅配额接力设置卡片。
 *
 * 所有支持本地路由的应用共用此卡片；Claude Desktop / GrokBuild 走各自独立的
 * 表单，也在那边引用它，因此单位换算与文案只有这一处实现。
 */
export function ProviderQuotaConfig({
  quota,
  onChange,
}: ProviderQuotaConfigProps) {
  const { t } = useTranslation();

  return (
    <div className="rounded-lg border border-border/50 bg-muted/20 p-4 space-y-4">
      <div className="flex items-center gap-3">
        <Gauge className="h-4 w-4 text-muted-foreground" />
        <span className="font-medium">
          {t("providerAdvanced.quotaRelay", { defaultValue: "订阅配额接力" })}
        </span>
      </div>
      <p className="text-sm text-muted-foreground">
        {t("providerAdvanced.quotaRelayDesc", {
          defaultValue:
            "开启本地路由后，本周期额度用尽会自动接力到下一个未超限的供应商。",
        })}
      </p>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="quota-max-tokens-cycle">
            {t("providerAdvanced.maxTokensCycle", {
              defaultValue: "周期 Token 上限",
            })}
            <span className="font-normal text-muted-foreground">
              {t("providerAdvanced.maxTokensCycleUnit", {
                defaultValue: "（单位：M）",
              })}
            </span>
          </Label>
          <Input
            id="quota-max-tokens-cycle"
            type="number"
            min="0"
            step="any"
            inputMode="decimal"
            value={rawTokensToM(quota.maxTokensCycle)}
            onChange={(e) =>
              onChange({
                ...quota,
                maxTokensCycle: mToRawTokens(e.target.value),
              })
            }
            placeholder={t("providerAdvanced.maxTokensCyclePlaceholder", {
              defaultValue: "如 1 表示 100 万",
            })}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="quota-cycle-duration-hours">
            {t("providerAdvanced.cycleDurationHours", {
              defaultValue: "周期时长（小时）",
            })}
          </Label>
          <Input
            id="quota-cycle-duration-hours"
            type="number"
            min="0"
            step="0.5"
            inputMode="decimal"
            value={quota.cycleDurationHours ?? ""}
            onChange={(e) =>
              onChange({
                ...quota,
                cycleDurationHours: e.target.value || undefined,
              })
            }
            placeholder={t("providerAdvanced.cycleDurationHoursPlaceholder", {
              defaultValue: "如 5",
            })}
          />
        </div>
        <div className="md:col-span-2 flex items-center justify-between">
          <div className="space-y-0.5">
            <Label htmlFor="quota-pay-as-you-go">
              {t("providerAdvanced.payAsYouGo", {
                defaultValue: "按量付费兜底",
              })}
            </Label>
            <p className="text-xs text-muted-foreground">
              {t("providerAdvanced.payAsYouGoHint", {
                defaultValue:
                  "所有周期额度耗尽时才会使用该供应商；周期额度恢复后自动切回。",
              })}
            </p>
          </div>
          <Switch
            id="quota-pay-as-you-go"
            checked={quota.payAsYouGo}
            onCheckedChange={(checked) =>
              onChange({ ...quota, payAsYouGo: checked })
            }
          />
        </div>
      </div>
    </div>
  );
}
