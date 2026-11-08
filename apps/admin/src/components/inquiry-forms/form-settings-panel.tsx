import type { InquiryFormField, InquiryFormSettings } from "@fastcart/shared-types";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

const MAX_NOTIFICATION_FIELDS = 6;

function SettingSwitch({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-background px-3.5 py-3">
      <div>
        <p className="text-[12px] font-medium text-foreground">{label}</p>
        <p className="mt-0.5 text-[10.5px] text-faint">{description}</p>
      </div>
      <Switch checked={checked} onCheckedChange={onChange} aria-label={label} />
    </div>
  );
}

export function FormSettingsPanel({
  settings,
  fields,
  onChange,
}: {
  settings: InquiryFormSettings;
  fields: InquiryFormField[];
  onChange: (settings: InquiryFormSettings) => void;
}) {
  const selectableFields = fields.filter(
    (field) => field.kind !== "file" && field.kind !== "hidden" && field.kind !== "consent",
  );

  const toggleNotificationField = (key: string) => {
    const selected = settings.notification_fields.includes(key);
    if (!selected && settings.notification_fields.length >= MAX_NOTIFICATION_FIELDS) return;
    onChange({
      ...settings,
      notification_fields: selected
        ? settings.notification_fields.filter((entry) => entry !== key)
        : [...settings.notification_fields, key],
    });
  };

  return (
    <div className="space-y-5">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor="settings-submit-label">Submit label</Label>
          <Input
            id="settings-submit-label"
            value={settings.submit_label}
            onChange={(event) => onChange({ ...settings, submit_label: event.target.value })}
            placeholder="Submit"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="settings-redirect">Redirect URL</Label>
          <Input
            id="settings-redirect"
            type="url"
            value={settings.redirect_url ?? ""}
            onChange={(event) =>
              onChange({
                ...settings,
                redirect_url: event.target.value.length > 0 ? event.target.value : null,
              })
            }
            placeholder="https://example.com/thank-you"
          />
        </div>
      </div>

      <div className="space-y-2">
        <Label htmlFor="settings-success">Success message</Label>
        <textarea
          id="settings-success"
          value={settings.success_message}
          onChange={(event) => onChange({ ...settings, success_message: event.target.value })}
          rows={2}
          className="w-full resize-y rounded-md border border-input bg-card px-3 py-2 text-[13px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25"
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="settings-consent">Consent text</Label>
        <textarea
          id="settings-consent"
          value={settings.consent_text ?? ""}
          onChange={(event) =>
            onChange({
              ...settings,
              consent_text: event.target.value.length > 0 ? event.target.value : null,
            })
          }
          rows={2}
          placeholder="I agree to be contacted about this enquiry."
          className="w-full resize-y rounded-md border border-input bg-card px-3 py-2 text-[13px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25"
        />
      </div>

      <div className="space-y-2">
        <SettingSwitch
          label="Consent required"
          description="Visitors must tick the consent box before submitting"
          checked={settings.consent_required}
          onChange={(checked) => onChange({ ...settings, consent_required: checked })}
        />
        <SettingSwitch
          label="Allow attachments"
          description="Show file fields and accept uploaded files"
          checked={settings.allow_attachments}
          onChange={(checked) => onChange({ ...settings, allow_attachments: checked })}
        />
      </div>

      <Separator />

      <div>
        <div className="flex items-baseline justify-between gap-2">
          <div>
            <p className="text-[12.5px] font-medium text-foreground">Notification fields</p>
            <p className="mt-0.5 text-[10.5px] text-faint">
              Include up to {MAX_NOTIFICATION_FIELDS} fields in new-inquiry notifications.
            </p>
          </div>
          <span className="font-mono text-[10.5px] text-faint">
            {settings.notification_fields.length}/{MAX_NOTIFICATION_FIELDS}
          </span>
        </div>

        {selectableFields.length === 0 ? (
          <p className="mt-3 rounded-lg border border-dashed border-border px-3 py-3 text-[11.5px] text-muted-foreground">
            Add a text, choice or contact field to make it available here.
          </p>
        ) : (
          <ul className="mt-3 grid gap-1.5 sm:grid-cols-2">
            {selectableFields.map((field) => {
              const checked = settings.notification_fields.includes(field.key);
              const disabled =
                !checked && settings.notification_fields.length >= MAX_NOTIFICATION_FIELDS;
              return (
                <li key={field.key}>
                  <label
                    className={cn(
                      "flex cursor-pointer items-center gap-2.5 rounded-lg border px-3 py-2 text-[12px] transition-colors",
                      checked
                        ? "border-brand/30 bg-brand-soft/40 text-foreground"
                        : "border-border bg-background text-ink-soft hover:border-line-strong",
                      disabled && "cursor-not-allowed opacity-50",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={checked}
                      disabled={disabled}
                      onChange={() => toggleNotificationField(field.key)}
                      className="size-3.5 accent-[var(--brand)]"
                    />
                    <span className="min-w-0 flex-1 truncate">{field.label}</span>
                    <span className="truncate font-mono text-[10px] text-faint">{field.key}</span>
                  </label>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
