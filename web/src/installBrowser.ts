export type Browser = "edge" | "chrome" | "firefox";

/** Tab panduan yang dibuka lebih dulu. Browser berbasis Chromium lain mengikuti Chrome. */
export function currentBrowser(userAgent: string): Browser {
  if (/\bEdg\//.test(userAgent)) return "edge";
  return /\bFirefox\//.test(userAgent) ? "firefox" : "chrome";
}
