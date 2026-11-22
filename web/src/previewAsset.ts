export async function isMissingPreviewAsset(error: unknown, signal: AbortSignal): Promise<boolean> {
  if (!(error instanceof Error)) return false;
  const source = /^(?:Failed to fetch dynamically imported module:|error loading dynamically imported module:|Unable to preload CSS for) (\S+)$/i.exec(error.message)?.[1];
  if (!source) return false;
  try {
    const url = new URL(source, window.location.origin);
    if (url.origin !== window.location.origin || !url.pathname.startsWith("/assets/") || !/\.(?:m?js|css)$/.test(url.pathname)) return false;
    const response = await fetch(url, { method: "HEAD", cache: "no-store", redirect: "error", signal });
    return response.status === 404;
  } catch {
    return false;
  }
}
