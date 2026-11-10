/** The line that sets up Herdr UI on another machine, for the Loft at `origin`. */
export function herdrInstallCommand(origin: string): string {
  const o = origin.replace(/\/+$/, "");
  return `curl -fsSL ${o}/api/herdr/install.sh | sh -s -- ${o}`;
}

/** An address only this machine can use, so a command built from it won't work elsewhere. */
export function isLocalOrigin(origin: string): boolean {
  try {
    const h = new URL(origin).hostname;
    return h === "localhost" || h === "::1" || h === "[::1]" || h.startsWith("127.");
  } catch {
    return false;
  }
}
