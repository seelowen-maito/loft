import { useState } from "react";
import { UserRound } from "lucide-react";
import { githubAvatarUrl, githubLogin } from "../lib/githubAvatar";

/** A GitHub user: their profile picture, and their login unless `bare`. If the picture can't load, the person icon and the login. */
export function GithubUser({ login, size = 14, bare = false }: { login: string; size?: number; bare?: boolean }) {
  const [failed, setFailed] = useState<string | null>(null);
  const url = githubAvatarUrl(login, size);
  if (!url) return null;
  const name = githubLogin(login);
  const pic = failed === url
    ? <UserRound className="shrink-0 text-fg-3" style={{ width: size, height: size }} aria-hidden />
    : <img src={url} alt="" width={size} height={size} onError={() => setFailed(url)} className="shrink-0 rounded-full bg-ink-850" style={{ width: size, height: size }} loading="lazy" />;
  return (
    <span className="inline-flex min-w-0 items-center gap-1" title={`Owner: ${name}`}>
      {pic}
      {(!bare || failed === url) && <span className="truncate font-mono">{name}</span>}
    </span>
  );
}
