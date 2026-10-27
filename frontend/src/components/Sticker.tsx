import { useMemo } from "react";
import type { Avatar } from "../lib/api";

/**
 * The Lead's captain hat, on the avatar's 100x100 canvas. Added over the art when the sticker is
 * drawn, inside its border, so the agent's own SVG never changes. iOS draws the same shapes.
 */
const CAPTAIN_HAT = '<g transform="rotate(-9 50 6)"><path d="M17 3 C15 -11 85 -11 83 3 L75 12 H25 Z" fill="#ffffff"/><path d="M20 6 L25 12 H75 L80 6 Z" fill="#dfe3ec"/><rect x="24" y="10" width="52" height="8" rx="2" fill="#1f2b5e"/><rect x="24" y="12.5" width="52" height="2" fill="#f2b632"/><path d="M24 17 H76 C73 26 27 26 24 17 Z" fill="#17171c"/><circle cx="50" cy="3" r="5.5" fill="#f2b632"/><circle cx="50" cy="3" r="2.4" fill="#1f2b5e"/></g>';

/**
 * Wraps an avatar's 100x100 drawing in a sticker: a white die-cut border grown
 * from its silhouette (blur the alpha, then threshold it back to a hard edge,
 * which rounds the outline and fills small notches) plus a soft drop shadow.
 * The canvas gets 14 units of room on each side for the border and shadow.
 */
export function stickerSvg(svg: string, lead = false) {
  const art = svg.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "") + (lead ? CAPTAIN_HAT : "");
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-14 -14 128 128">
<defs><filter id="hb-sticker" x="-14" y="-14" width="128" height="128" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
<feGaussianBlur in="SourceAlpha" stdDeviation="3.4"/>
<feComponentTransfer result="cut"><feFuncA type="linear" slope="36" intercept="-0.9"/></feComponentTransfer>
<feGaussianBlur in="cut" stdDeviation="2.2"/><feOffset dy="2" result="blur"/>
<feFlood flood-color="#000" flood-opacity="0.32"/><feComposite in2="blur" operator="in" result="shadow"/>
<feFlood flood-color="#fff"/><feComposite in2="cut" operator="in" result="paper"/>
<feMerge><feMergeNode in="shadow"/><feMergeNode in="paper"/><feMergeNode in="SourceGraphic"/></feMerge>
</filter></defs><g filter="url(#hb-sticker)">${art}</g></svg>`;
}

/**
 * An agent's avatar as a sticker, with the captain hat when it is its team's Lead. Rendered through <img>, so the SVG can never
 * run scripts or load anything, and it stays crisp at any size.
 */
export function Sticker({ avatar, lead = false, size = 40, className = "" }: { avatar: Avatar; lead?: boolean; size?: number; className?: string }) {
  const src = useMemo(() => "data:image/svg+xml;charset=utf-8," + encodeURIComponent(stickerSvg(avatar.svg, lead)), [avatar.svg, lead]);
  return <img src={src} alt="" draggable={false} width={size} height={size} className={`sticker shrink-0 select-none ${className}`} style={{ width: size, height: size }} />;
}
