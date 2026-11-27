function versionParts(version: string | null) {
  const parts = version?.split(".");
  if (parts?.length !== 3 || parts.some((part) => !part || /\D/.test(part))) return null;
  const numbers = parts.map(Number);
  return numbers.every(Number.isSafeInteger) ? numbers : null;
}

export function extensionUpdateAvailable(installed: string | null, latest: string): boolean {
  const current = versionParts(installed);
  const available = versionParts(latest);
  if (!available) return false;
  if (!current) return true;
  const firstDifference = current.findIndex((part, index) => part !== available[index]);
  return firstDifference >= 0 && current[firstDifference] < available[firstDifference];
}
