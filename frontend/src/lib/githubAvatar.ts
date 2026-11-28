/** A GitHub login as typed: trimmed, without a leading `@`. */
export const githubLogin = (login: string) => login.trim().replace(/^@/, "");

/** The GitHub profile picture of a login, at `size` pixels (asked at twice that for sharp screens); null with no login. */
export function githubAvatarUrl(login: string, size: number): string | null {
  const name = githubLogin(login);
  return name ? `https://github.com/${encodeURIComponent(name)}.png?size=${size * 2}` : null;
}
