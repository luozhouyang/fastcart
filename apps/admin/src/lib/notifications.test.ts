import { describe, expect, it } from "vitest";

import { channelEndpointLabel, channelSecretLabel, maskEndpoint } from "@/lib/notifications";

describe("notification channel display", () => {
  it("shows the server-masked endpoint and falls back safely", () => {
    expect(
      channelEndpointLabel({
        endpoint_url_masked: "https://open.feishu.cn/open-apis/bot/v2/hook/••••3f2a",
      }),
    ).toBe("https://open.feishu.cn/open-apis/bot/v2/hook/••••3f2a");
    expect(channelEndpointLabel({ endpoint_url_masked: "" })).toBe("Endpoint hidden");
    expect(channelEndpointLabel(null)).toBe("Endpoint hidden");
  });

  it("never renders a secret value", () => {
    expect(channelSecretLabel(true)).toBe("Signing secret configured");
    expect(channelSecretLabel(false)).toBe("No signing secret");
  });

  it("masks the trailing path of a webhook URL", () => {
    const masked = maskEndpoint("https://open.feishu.cn/open-apis/bot/v2/hook/0123456789abcdef");
    expect(masked).toContain("https://open.feishu.cn");
    expect(masked).toContain("••••");
    expect(masked).not.toContain("0123456789abcdef");
  });
});
