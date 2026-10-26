import { useMemo } from "react";
import type { Avatar } from "../lib/api";

/**
 * Wraps an avatar's 100x100 drawing in a sticker: a white die-cut border grown
 * from its silhouette (blur the alpha, then threshold it back to a hard edge,
 * which rounds the outline and fills small notches) plus a soft drop shadow.
 * The canvas gets 14 units of room on each side for the border and shadow.
 */
export function stickerSvg(svg: string) {
  const art = svg.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "");
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
 * An agent's avatar as a sticker. Rendered through <img>, so the SVG can never
 * run scripts or load anything, and it stays crisp at any size.
 */
export function Sticker({ avatar, size = 40, className = "" }: { avatar: Avatar; size?: number; className?: string }) {
  const src = useMemo(() => "data:image/svg+xml;charset=utf-8," + encodeURIComponent(stickerSvg(avatar.svg)), [avatar.svg]);
  return <img src={src} alt="" draggable={false} width={size} height={size} className={`sticker shrink-0 select-none ${className}`} style={{ width: size, height: size }} />;
}
