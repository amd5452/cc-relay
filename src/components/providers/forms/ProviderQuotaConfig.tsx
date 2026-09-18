import { useTranslation } from "react-i18next";
import { Gauge } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

/** 订阅配额接力配置（对应 providers 表的真实列）。 */
export interface ProviderQuotaConfigValue {
  maxTokensCycle?: string;
  cycleDurationHours?: string;
  payAsYouGo: boolean;
}

interface ProviderQuotaConfigProps {
  quota: ProviderQuotaConfigValue;
  onChange: (quota: ProviderQuotaConfigValue) => void;
}

/**
 * 订阅配额接力设置卡片。
 *
 * 独立于「计费配置」：增量模式应用（OpenCode / OpenClaw / Hermes）不展示计费
 * 配置，但仍需要按周期额度做接力，因此本卡片单独渲染，不走 ProviderAdvancedConfig。
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
          </Label>
          <Input
            id="quota-max-tokens-cycle"
            type="number"
            min="0"
            inputMode="numeric"
            value={quota.maxTokensCycle ?? ""}
            onChange={(e) =>
              onChange({
                ...quota,
                maxTokensCycle: e.target.value || undefined,
              })
            }
            placeholder={t("providerAdvanced.maxTokensCyclePlaceholder", {
              defaultValue: "留空表示不限量",
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
