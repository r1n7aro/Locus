import { describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import {
  CUSTOM_ENDPOINT_HTML_RESPONSE_CODE,
  customEndpointTestDetail,
  customEndpointTestHtmlPath,
  customEndpointTestStatusForReply,
  isCustomEndpointHtmlResponse,
} from "../services/customEndpointTestResult";

describe("custom endpoint test result", () => {
  it("classifies HTML challenge responses as endpoint test failures", () => {
    expect(isCustomEndpointHtmlResponse({
      code: CUSTOM_ENDPOINT_HTML_RESPONSE_CODE,
      message: "Server returned an HTML page instead of JSON.",
    })).toBe(true);
    expect(isCustomEndpointHtmlResponse(
      "Server returned an HTML page instead of JSON (possible verification/challenge page). [OPEN_HTML:C:\\Temp\\test.html]",
    )).toBe(true);
    expect(isCustomEndpointHtmlResponse("pong")).toBe(false);
    expect(customEndpointTestStatusForReply(
      "Server returned an HTML page instead of JSON. [OPEN_HTML:C:\\Temp\\test.html]",
    )).toBe("error");
    expect(customEndpointTestStatusForReply("pong")).toBe("success");
  });

  it("localizes HTML response details and preserves the saved HTML path", () => {
    const result = "HTTP 403 — Server returned an HTML page instead of JSON. [OPEN_HTML:C:\\Temp\\test.html]";

    setLocale("zh");
    expect(customEndpointTestDetail(result)).toBe(
      "HTTP 403 — 服务器返回了 HTML 页面，未返回 JSON，可能是验证或挑战页面。",
    );

    setLocale("en");
    expect(customEndpointTestDetail(result)).toBe(
      "HTTP 403 — Server returned an HTML page instead of JSON, possibly a verification or challenge page.",
    );
    expect(customEndpointTestHtmlPath(result)).toBe("C:\\Temp\\test.html");
  });
});
