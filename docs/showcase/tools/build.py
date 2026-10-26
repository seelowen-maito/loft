"""Build the Loft showcase: one self-contained web page and a print page for the PDF.

usage: python3 build.py <shots dir> <out dir>
  shots/web/web-*.png        browser screenshots (dark)
  shots/ios/{light,dark}/<key>.png   iPhone screenshots, keys as in content.PHONE_CAPTIONS
"""
import base64
import html
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from content import COMING_SOON, INTRO, PHONE_CAPTIONS, SECTIONS, TAGLINE  # noqa: E402

SHOTS, OUT = Path(sys.argv[1]), Path(sys.argv[2])
FONTS = Path(__file__).parent / "fonts"
OUT.mkdir(parents=True, exist_ok=True)
TMP = Path(tempfile.mkdtemp())
_cache = {}


def webp(src: Path, width: int, q=78) -> str:
    """A data: URI of src resized to width, as WebP."""
    key = (src, width)
    if key not in _cache:
        dst = TMP / f"{src.stem}-{src.parent.name}-{width}.webp"
        subprocess.run(["cwebp", "-quiet", "-q", str(q), "-resize", str(width), "0", str(src), "-o", str(dst)], check=True)
        _cache[key] = "data:image/webp;base64," + base64.b64encode(dst.read_bytes()).decode()
    return _cache[key]


def web_img(name, width=1600):
    p = SHOTS / "web" / f"{name}.png"
    return webp(p, width) if p.exists() else None


def phone_img(key, scheme, width=560):
    p = SHOTS / "ios" / scheme / f"{key}.png"
    if not p.exists():
        p = SHOTS / "ios" / ("dark" if scheme == "light" else "light") / f"{key}.png"
    return webp(p, width) if p.exists() else None


def font_face():
    """Instrument Serif (SIL OFL), inlined so the page makes no outside requests."""
    css = ""
    for style, f in (("normal", "InstrumentSerif-Regular.woff2"), ("italic", "InstrumentSerif-Italic.woff2")):
        data = base64.b64encode((FONTS / f).read_bytes()).decode()
        css += f"@font-face{{font-family:'Instrument Serif';font-style:{style};font-weight:400;font-display:swap;src:url(data:font/woff2;base64,{data}) format('woff2')}}"
    return css


def e(s):
    return html.escape(s, quote=True)


def browser(name, cls=""):
    src = web_img(name)
    if not src:
        return ""
    return f'''<figure class="browser {cls}"><div class="bar"><i></i><i></i><i></i><span>loft</span></div><img src="{src}" alt="" loading="lazy"></figure>'''


def phone(key, cls=""):
    light, dark = phone_img(key, "light"), phone_img(key, "dark")
    if not (light or dark):
        inner = f'<div class="ph-missing">{e(PHONE_CAPTIONS.get(key, key))}</div>'
    else:
        inner = f'<img class="only-light" src="{light or dark}" alt="" loading="lazy"><img class="only-dark" src="{dark or light}" alt="" loading="lazy">'
    return f'<figure class="phone {cls}"><div class="screen">{inner}</div><figcaption>{e(PHONE_CAPTIONS.get(key, ""))}</figcaption></figure>'


CHECK = '<svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 10.5l3.2 3L15 6.5" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>'
LOGO = '<svg viewBox="0 0 32 32" aria-hidden="true"><path d="M16 3 3 13v16h9v-9h8v9h9V13z" fill="var(--accent)"/><rect x="13" y="22" width="6" height="7" rx="1" fill="var(--bg)"/></svg>'

# ---------------------------------------------------------------- web page

def sections_html():
    out = []
    for i, (key, eyebrow, title, lead, bullets, webs, phones) in enumerate(SECTIONS):
        media = "".join(browser(w, f"b{j}") for j, w in enumerate(webs))
        media += "".join(phone(p, f"p{j}") for j, p in enumerate(phones))
        layout = "only-phones" if not webs else ("with-phone" if phones else "only-web")
        lis = "".join(f'<li class="reveal" style="--d:{120 + 70 * j}ms">{CHECK}<span>{e(b)}</span></li>' for j, b in enumerate(bullets))
        out.append(f'''
<section id="{key}" class="feature {'flip' if i % 2 else ''}">
  <div class="copy">
    <p class="eyebrow reveal"><span class="num">{i + 1:02d}</span>{e(eyebrow)}</p>
    <h2 class="reveal" style="--d:60ms">{e(title)}</h2>
    <p class="lead reveal" style="--d:100ms">{e(lead)}</p>
    <ul class="bullets">{lis}</ul>
  </div>
  <div class="media {layout} reveal-media">{media}</div>
</section>''')
    return "\n".join(out)


HERO_PHONES = ["overview", "chat", "needs-you", "talk"]


def hero_phone():
    imgs = []
    for j, k in enumerate(HERO_PHONES):
        l, d = phone_img(k, "light"), phone_img(k, "dark")
        if l or d:
            imgs.append(f'<img class="only-light slide" style="--i:{j}" src="{l or d}" alt=""><img class="only-dark slide" style="--i:{j}" src="{d or l}" alt="">')
    inner = "".join(imgs) or '<div class="ph-missing">Loft on iPhone</div>'
    return f'<figure class="phone hero-phone"><div class="screen cycler" style="--n:{max(1, len(imgs))}">{inner}</div></figure>'


CSS = r"""
:root{--bg:#f6f7f2;--bg2:#ecefe4;--card:#ffffff;--ink:#14160f;--muted:#5b6152;--line:#d9ddcf;--accent:#5d8f00;--accent-ink:#ffffff;--glow:rgba(140,200,40,.25);--frame:#1b1d18;--shadow:0 30px 60px -20px rgba(20,30,0,.25),0 10px 20px -10px rgba(20,30,0,.15)}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){--bg:#0b0c0a;--bg2:#121410;--card:#16180f;--ink:#eef1e6;--muted:#9aa18d;--line:#262a20;--accent:#c4f25a;--accent-ink:#11140a;--glow:rgba(196,242,90,.18);--frame:#2a2d25;--shadow:0 30px 70px -20px rgba(0,0,0,.7),0 10px 30px -10px rgba(0,0,0,.5)}}
:root[data-theme="dark"]{--bg:#0b0c0a;--bg2:#121410;--card:#16180f;--ink:#eef1e6;--muted:#9aa18d;--line:#262a20;--accent:#c4f25a;--accent-ink:#11140a;--glow:rgba(196,242,90,.18);--frame:#2a2d25;--shadow:0 30px 70px -20px rgba(0,0,0,.7),0 10px 30px -10px rgba(0,0,0,.5)}
*{box-sizing:border-box}
html{scroll-behavior:smooth;-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--ink);font:17px/1.6 ui-sans-serif,-apple-system,BlinkMacSystemFont,"SF Pro Text","Inter","Segoe UI",sans-serif;overflow-x:hidden;transition:background .4s,color .4s}
.serif{font-family:"Instrument Serif","New York",ui-serif,Georgia,serif;font-weight:400}
a{color:inherit}
.only-dark{display:none}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]) .only-dark{display:block}:root:not([data-theme="light"]) .only-light{display:none}}
:root[data-theme="dark"] .only-dark{display:block}:root[data-theme="dark"] .only-light{display:none}
:root[data-theme="light"] .only-dark{display:none}:root[data-theme="light"] .only-light{display:block}

/* nav */
nav.top{position:fixed;inset:0 0 auto;z-index:50;display:flex;align-items:center;gap:20px;padding:14px max(16px,calc((100vw - 1180px)/2));background:color-mix(in srgb,var(--bg) 78%,transparent);backdrop-filter:saturate(1.4) blur(14px);-webkit-backdrop-filter:saturate(1.4) blur(14px);border-bottom:1px solid transparent;transition:border-color .3s}
nav.top.scrolled{border-color:var(--line)}
.brand{display:flex;align-items:center;gap:10px;font-weight:600;letter-spacing:-.01em;text-decoration:none;flex:none}
.brand svg{width:26px;height:26px}
.links{display:flex;gap:4px;overflow-x:auto;scrollbar-width:none;flex:1;min-width:0;mask-image:linear-gradient(90deg,#000 85%,transparent)}
.links::-webkit-scrollbar{display:none}
.links a{white-space:nowrap;text-decoration:none;color:var(--muted);font-size:14px;padding:6px 12px;border-radius:99px;transition:color .2s,background .2s}
.links a:hover,.links a.active{color:var(--ink);background:var(--bg2)}
.theme{flex:none;width:38px;height:38px;border-radius:99px;border:1px solid var(--line);background:var(--card);color:var(--ink);cursor:pointer;display:grid;place-items:center}
.theme svg{width:18px;height:18px}

/* hero */
.hero{position:relative;isolation:isolate;min-height:100svh;display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);align-items:center;gap:40px;padding:120px max(16px,calc((100vw - 1180px)/2)) 60px;overflow:hidden}
.hero::before{content:"";position:absolute;inset:-20% -10% auto auto;width:70vw;height:70vw;background:radial-gradient(closest-side,var(--glow),transparent);filter:blur(10px);animation:drift 14s ease-in-out infinite alternate;pointer-events:none;z-index:-1}
.grid-bg{position:absolute;inset:0;background-image:linear-gradient(var(--line) 1px,transparent 1px),linear-gradient(90deg,var(--line) 1px,transparent 1px);background-size:56px 56px;mask-image:radial-gradient(ellipse at 30% 40%,#000 0%,transparent 65%);opacity:.5;pointer-events:none;z-index:-1}
@keyframes drift{to{transform:translate(-8vw,10vh) scale(1.1)}}
.hero h1{font-size:clamp(44px,5.6vw,80px);line-height:.98;letter-spacing:-.025em;margin:0 0 24px}
.hero h1 em{color:var(--accent);font-style:italic}
.hero p.sub{font-size:clamp(17px,1.6vw,20px);color:var(--muted);max-width:560px;margin:0 0 32px}
.pill{display:inline-flex;align-items:center;gap:8px;font-size:13px;color:var(--muted);border:1px solid var(--line);background:var(--card);border-radius:99px;padding:6px 12px 6px 8px;margin-bottom:28px}
.pill b{background:var(--accent);color:var(--accent-ink);border-radius:99px;padding:2px 8px;font-size:12px}
.ctas{display:flex;gap:12px;flex-wrap:wrap}
.btn{display:inline-flex;align-items:center;gap:8px;text-decoration:none;font-weight:600;padding:13px 22px;border-radius:99px;border:1px solid var(--line);background:var(--card);transition:transform .2s}
.btn.primary{background:var(--accent);color:var(--accent-ink);border-color:transparent}
.btn:hover{transform:translateY(-2px)}
.platforms{display:flex;gap:18px;margin-top:36px;color:var(--muted);font-size:14px;flex-wrap:wrap}
.platforms span{display:flex;align-items:center;gap:8px}
.platforms i{width:8px;height:8px;border-radius:99px;background:var(--accent);box-shadow:0 0 0 4px var(--glow)}
.hero-stage{position:relative;height:min(78vh,700px);perspective:1400px}
.hero-stage .browser{position:absolute;width:min(720px,58vw);right:-22%;top:6%;transform:rotateY(-14deg) rotateX(4deg);animation:float 9s ease-in-out infinite}
.hero-phone{position:absolute!important;left:6%;top:4%;width:min(300px,62%)!important;animation:float 7s ease-in-out infinite reverse}
@keyframes float{50%{translate:0 -14px}}
.cycler{position:relative}
.cycler img.slide{position:absolute;inset:0;opacity:0;animation:cycle calc(var(--n)*3.2s) infinite;animation-delay:calc(var(--i)*3.2s)}
.cycler img.slide[style*="--i:0"]{position:relative}
@keyframes cycle{0%{opacity:0;transform:scale(1.03)}5%{opacity:1;transform:none}30%{opacity:1}31%{opacity:0}100%{opacity:0}}
.scroll-hint{position:absolute;left:50%;bottom:24px;translate:-50% 0;width:24px;height:38px;border:2px solid var(--line);border-radius:14px}
.scroll-hint::after{content:"";position:absolute;left:50%;top:7px;width:4px;height:8px;margin-left:-2px;border-radius:2px;background:var(--muted);animation:wheel 1.8s infinite}
@keyframes wheel{to{transform:translateY(12px);opacity:0}}

/* devices */
.browser{margin:0;border-radius:14px;overflow:hidden;background:var(--frame);box-shadow:var(--shadow);border:1px solid var(--line)}
.browser .bar{display:flex;align-items:center;gap:7px;padding:10px 14px;background:var(--frame)}
.browser .bar i{width:11px;height:11px;border-radius:99px;background:#ff5f57}.browser .bar i:nth-child(2){background:#febc2e}.browser .bar i:nth-child(3){background:#28c840}
.browser .bar span{margin:0 auto;font-size:12px;color:#9aa18d;background:rgba(255,255,255,.06);padding:3px 40px;border-radius:6px;translate:-20px 0}
.browser img{display:block;width:100%;height:auto}
.phone{margin:0;position:relative;width:260px;flex:none}
.phone .screen{position:relative;border-radius:44px;overflow:hidden;border:10px solid var(--frame);background:var(--frame);box-shadow:var(--shadow);aspect-ratio:1206/2622}
.phone .screen::before{content:"";position:absolute;z-index:3;top:10px;left:50%;translate:-50% 0;width:32%;height:26px;border-radius:99px;background:#000}
.phone img{display:block;width:100%;height:100%;object-fit:cover}
.phone figcaption{text-align:center;font-size:13px;color:var(--muted);margin-top:12px}
.hero-phone figcaption{display:none}
.ph-missing{position:absolute;inset:0;display:grid;place-items:center;text-align:center;padding:24px;color:var(--muted);background:linear-gradient(160deg,var(--bg2),var(--card));font-size:14px}

/* features */
.feature{display:grid;grid-template-columns:.85fr 1.15fr;gap:64px;align-items:center;padding:120px max(16px,calc((100vw - 1180px)/2));position:relative}
.feature.flip{grid-template-columns:1.15fr .85fr}
.feature.flip .copy{order:2}
.feature+.feature{border-top:1px solid var(--line)}
.eyebrow{display:flex;align-items:center;gap:12px;font-size:13px;letter-spacing:.14em;text-transform:uppercase;color:var(--muted);margin:0 0 18px}
.eyebrow .num{font-family:ui-monospace,"SF Mono",Menlo,monospace;color:var(--accent);letter-spacing:0}
.feature h2{font-size:clamp(34px,4.4vw,56px);line-height:1.04;letter-spacing:-.02em;margin:0 0 20px;font-family:"Instrument Serif","New York",ui-serif,Georgia,serif;font-weight:400}
.lead{font-size:19px;color:var(--muted);margin:0 0 28px}
.bullets{list-style:none;padding:0;margin:0;display:grid;gap:14px}
.bullets li{display:flex;gap:12px;align-items:flex-start}
.bullets svg{flex:none;width:22px;height:22px;padding:3px;margin-top:2px;border-radius:99px;background:var(--accent);color:var(--accent-ink)}
.media{position:relative;display:flex;align-items:center;justify-content:center;min-height:420px}
.media.with-phone .browser{width:100%}
.media.with-phone .browser.b1{display:none}
.media.with-phone .phone{position:absolute;right:-24px;bottom:-40px;width:220px}
.media.with-phone .phone.p1{right:auto;left:-30px;bottom:-60px;width:190px}
.media.with-phone .phone.p1 figcaption{display:none}
.media.only-web{flex-direction:column;align-items:stretch}
.media.only-web .browser.b1{width:46%;position:absolute;right:-30px;bottom:-60px}
.media.only-phones{gap:28px}
.media.only-phones .phone:nth-child(2){translate:0 60px}

/* reveal motion */
.js .reveal{opacity:0;transform:translateY(24px);transition:opacity .8s cubic-bezier(.2,.7,.2,1) var(--d,0ms),transform .8s cubic-bezier(.2,.7,.2,1) var(--d,0ms)}
.js .reveal.in{opacity:1;transform:none}
.js .reveal-media>*{opacity:0;transform:translateY(60px) scale(.96);transition:opacity 1s cubic-bezier(.2,.7,.2,1),transform 1.1s cubic-bezier(.2,.7,.2,1)}
.js .reveal-media>.phone{transform:translate(60px,80px) rotate(4deg)}
.js .reveal-media>.phone.p1{transform:translate(-60px,80px) rotate(-4deg)}
.js .reveal-media>*:nth-child(2){transition-delay:.15s}.js .reveal-media>*:nth-child(3){transition-delay:.3s}
.js .reveal-media.in>*,.js .reveal-media.in>.phone,.js .reveal-media.in>.phone.p1{opacity:1;transform:none}
.media .phone,.media .browser{will-change:transform}

/* coming soon + footer */
.soon{padding:110px max(16px,calc((100vw - 1180px)/2));border-top:1px solid var(--line);text-align:center}
.soon h2{font-size:clamp(32px,4vw,48px);margin:0 0 40px;letter-spacing:-.02em}
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:20px;text-align:left}
.card{background:var(--card);border:1px solid var(--line);border-radius:20px;padding:26px}
.card b{display:block;font-size:19px;margin-bottom:6px}
.card span{color:var(--muted)}
.tag{display:inline-block;font-size:12px;letter-spacing:.1em;text-transform:uppercase;color:var(--accent);margin-bottom:12px}
footer{padding:80px 16px 60px;text-align:center;color:var(--muted);border-top:1px solid var(--line)}
footer .big{font-size:clamp(36px,6vw,72px);color:var(--ink);line-height:1;margin:0 0 18px}
footer .big em{color:var(--accent)}
.progress{position:fixed;left:0;top:0;height:2px;background:var(--accent);z-index:60;width:0}

@media (max-width:900px){
  body{font-size:16px}
  .hero{grid-template-columns:1fr;padding-top:100px;min-height:auto}
  .hero-stage{height:520px;margin-top:10px}
  .hero-stage .browser{width:88vw;right:-30vw;top:16%}
  .hero-phone{width:230px!important;left:0}
  .feature,.feature.flip{grid-template-columns:1fr;gap:40px;padding:80px 16px}
  .feature.flip .copy{order:0}
  .media{min-height:0;padding-bottom:40px}
  .media.with-phone{padding:0 0 80px}
  .media.with-phone .phone{width:150px;right:-6px;bottom:0}
  .media.with-phone .phone.p1{display:none}
  .media.only-web .browser.b1{width:58%;right:-8px;bottom:-30px}
  .media.only-phones{gap:14px}
  .media.only-phones .phone{width:46%}
  .media.only-phones .phone:nth-child(2){translate:0 40px}
  .phone .screen{border-width:6px;border-radius:30px}
  .phone .screen::before{top:6px;height:16px}
  .scroll-hint{display:none}
}
@media (prefers-reduced-motion:reduce){
  *,*::before,*::after{animation:none!important;transition:none!important}
  .js .reveal,.js .reveal-media>*{opacity:1;transform:none}
  .cycler img.slide{opacity:0}.cycler img.slide[style*="--i:0"]{opacity:1}
}
"""

JS = r"""
document.documentElement.classList.add('js');
const io=new IntersectionObserver(es=>es.forEach(e=>{if(e.isIntersecting){e.target.classList.add('in');io.unobserve(e.target)}}),{threshold:.15,rootMargin:'0px 0px -8% 0px'});
document.querySelectorAll('.reveal,.reveal-media').forEach(el=>io.observe(el));
const nav=document.querySelector('nav.top'),bar=document.querySelector('.progress'),links=[...document.querySelectorAll('.links a')];
const secs=links.map(a=>document.querySelector(a.getAttribute('href')));
const reduce=matchMedia('(prefers-reduced-motion: reduce)').matches;
let ticking=false;
function onScroll(){ticking=false;const y=scrollY,h=document.documentElement.scrollHeight-innerHeight;
  nav.classList.toggle('scrolled',y>10);bar.style.width=(h>0?y/h*100:0)+'%';
  let cur=-1;secs.forEach((s,i)=>{if(s&&s.getBoundingClientRect().top<innerHeight*.4)cur=i});
  links.forEach((a,i)=>a.classList.toggle('active',i===cur));
  if(cur<0&&links[0].parentElement.scrollLeft>0)links[0].parentElement.scrollTo({left:0,behavior:'smooth'});if(cur>=0){const a=links[cur],box=a.parentElement;const l=a.offsetLeft-box.clientWidth/2+a.clientWidth/2;if(Math.abs(box.scrollLeft-l)>40)box.scrollTo({left:l,behavior:'smooth'})}
  if(!reduce)document.querySelectorAll('.media.in .phone').forEach(p=>{const r=p.getBoundingClientRect();const t=(r.top+r.height/2-innerHeight/2)/innerHeight;p.style.translate=`0 ${(t*-30).toFixed(1)}px`});
}
addEventListener('scroll',()=>{if(!ticking){ticking=true;requestAnimationFrame(onScroll)}},{passive:true});onScroll();
const root=document.documentElement,btn=document.querySelector('.theme');
function setTheme(t){root.dataset.theme=t;try{localStorage.setItem('hb-theme',t)}catch(e){}}
try{const t=localStorage.getItem('hb-theme');if(t)root.dataset.theme=t}catch(e){}
btn.addEventListener('click',()=>{const dark=root.dataset.theme?root.dataset.theme==='dark':matchMedia('(prefers-color-scheme: dark)').matches;setTheme(dark?'light':'dark')});
"""

SUN = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>'


def page():
    nav_links = "".join(f'<a href="#{k}">{e(eb)}</a>' for k, eb, *_ in SECTIONS)
    soon = "".join(f'<div class="card reveal" style="--d:{i * 90}ms"><span class="tag">Coming soon</span><b>{e(t)}</b><span>{e(d)}</span></div>' for i, (t, d) in enumerate(COMING_SOON))
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>Loft</title>
<meta name="description" content="{e(TAGLINE)}">
<meta name="color-scheme" content="light dark">
<style>{font_face()}{CSS}</style></head>
<body>
<div class="progress"></div>
<nav class="top"><a class="brand" href="#top">{LOGO}loft</a><div class="links">{nav_links}</div><button class="theme" aria-label="Switch light or dark">{SUN}</button></nav>
<header class="hero" id="top">
  <div class="grid-bg"></div>
  <div>
    <div class="pill reveal"><b>New</b> Talk to your agents, hands-free</div>
    <h1 class="serif reveal" style="--d:80ms">Your own crew of AI developers, <em>working from your Mac.</em></h1>
    <p class="sub reveal" style="--d:160ms">{e(INTRO)}</p>
    <div class="ctas reveal" style="--d:240ms"><a class="btn primary" href="#crew">See what it does</a><a class="btn" href="#voice">Hear it talk</a></div>
    <div class="platforms reveal" style="--d:320ms"><span><i></i>Runs on your Mac</span><span><i></i>Web app</span><span><i></i>iPhone app</span></div>
  </div>
  <div class="hero-stage">{browser("web-chat-orchestrator")}{hero_phone()}</div>
  <div class="scroll-hint"></div>
</header>
<main>{sections_html()}
<section class="soon"><h2 class="serif reveal">What's next</h2><div class="cards">{soon}</div></section>
</main>
<footer><p class="big serif reveal">Less typing. <em>More building.</em></p><p>Loft, built with Claude.</p></footer>
<script>{JS}</script>
</body></html>"""


# ---------------------------------------------------------------- PDF (print page)

PRINT_CSS = r"""
@page{size:297mm 210mm;margin:0}
*{box-sizing:border-box}
body{margin:0;font:11.5pt/1.5 -apple-system,"SF Pro Text","Helvetica Neue",sans-serif;color:#14160f;-webkit-print-color-adjust:exact;print-color-adjust:exact}
.serif{font-family:"Instrument Serif","New York",Georgia,serif;font-weight:400}
.pg{width:297mm;height:210mm;position:relative;overflow:hidden;page-break-after:always;padding:16mm 16mm 14mm;background:#f6f7f2}
.pg.dark{background:#0b0c0a;color:#eef1e6}
.foot{position:absolute;left:16mm;right:16mm;bottom:8mm;display:flex;justify-content:space-between;font-size:8.5pt;color:#8a907e}
.cover{display:grid;grid-template-columns:1.05fr .95fr;gap:10mm;align-items:center}
.cover::before{content:"";position:absolute;right:-60mm;top:-60mm;width:200mm;height:200mm;background:radial-gradient(closest-side,rgba(196,242,90,.22),transparent)}
.cover h1{font-size:50pt;line-height:1;letter-spacing:-.02em;margin:6mm 0 7mm}
.cover h1 em{color:#c4f25a}
.cover p{color:#9aa18d;font-size:13pt;max-width:120mm}
.brand{display:flex;align-items:center;gap:3mm;font-weight:600;font-size:14pt}
.brand svg{width:9mm;height:9mm}
.chips{display:flex;gap:3mm;margin-top:8mm}
.chips span{border:1px solid #2a2d25;border-radius:99px;padding:1.5mm 4mm;font-size:9.5pt;color:#c9cfbb}
.stage{position:relative;height:150mm}
.stage .browser{position:absolute;right:-30mm;top:14mm;width:150mm}
.stage .phone{position:absolute;left:0;top:0;width:66mm}
.feat{display:grid;grid-template-columns:96mm minmax(0,1fr);gap:12mm;align-items:center;height:100%}
.eyebrow{font-size:9pt;letter-spacing:.14em;text-transform:uppercase;color:#6c7360;margin:0 0 4mm;display:flex;gap:3mm}
.eyebrow b{color:#5d8f00;font-family:Menlo,monospace;letter-spacing:0}
h2{font-size:28pt;line-height:1.05;margin:0 0 5mm;letter-spacing:-.015em}
.lead{color:#4e5446;margin:0 0 5mm;font-size:12pt}
ul{list-style:none;margin:0;padding:0;display:grid;gap:2.6mm}
li{display:flex;gap:3mm;font-size:10.5pt}
li::before{content:"";flex:none;width:4.2mm;height:4.2mm;margin-top:.8mm;border-radius:99px;background:#5d8f00 url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20'%3E%3Cpath d='M5 10.5l3.2 3L15 6.5' fill='none' stroke='white' stroke-width='2.6' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E") center/80% no-repeat}
.shots{position:relative;height:170mm;display:flex;align-items:center;justify-content:center;gap:7mm}
.shots .browser{width:100%}
.shots.with-phone .browser{width:150mm;margin-right:24mm}
.shots.with-phone .phone{position:absolute;right:0;bottom:4mm;width:52mm}
.shots.with-phone .phone.p1{display:none}
.shots.with-phone .browser.b1{display:none}
.shots.only-web .browser.b1{display:none}
.shots.only-phones .phone{width:62mm}
.browser{margin:0;border-radius:3mm;overflow:hidden;background:#1b1d18;box-shadow:0 8mm 16mm -6mm rgba(20,30,0,.35)}
.browser .bar{display:flex;gap:1.6mm;padding:2.2mm 3mm}
.browser .bar i{width:2.4mm;height:2.4mm;border-radius:99px;background:#ff5f57}.browser .bar i:nth-child(2){background:#febc2e}.browser .bar i:nth-child(3){background:#28c840}
.browser .bar span{display:none}
.browser img{display:block;width:100%}
.phone{margin:0;width:60mm}
.phone .screen{border:2.4mm solid #1b1d18;border-radius:9mm;overflow:hidden;background:#1b1d18;aspect-ratio:1206/2622;box-shadow:0 8mm 16mm -6mm rgba(20,30,0,.35);position:relative}
.phone img{display:block;width:100%;height:100%;object-fit:cover}
.phone .only-dark{display:none}
.phone figcaption{text-align:center;font-size:8.5pt;color:#6c7360;margin-top:2.5mm}
.ph-missing{position:absolute;inset:0;display:grid;place-items:center;color:#6c7360;background:#ecefe4;font-size:9pt}
.toc{display:grid;grid-template-columns:repeat(4,1fr);gap:4mm;margin-top:7mm}
.toc div{background:#fff;border:1px solid #d9ddcf;border-radius:4mm;padding:5mm}
.toc b{display:block;color:#5d8f00;font-family:Menlo,monospace;font-size:9pt;margin-bottom:2mm}
.toc span{font-size:11.5pt;font-weight:600;line-height:1.3;display:block}
.toc p{font-size:8.6pt;color:#5b6152;margin:2mm 0 0;line-height:1.4}
.soon{display:grid;grid-template-columns:1fr 1fr;gap:6mm;margin-top:8mm}
.soon div{border:1px solid #2a2d25;border-radius:4mm;padding:6mm;background:#16180f}
.soon b{display:block;font-size:14pt;margin-bottom:2mm}
.soon span{color:#9aa18d}
"""


def print_page():
    pages = [f'''<div class="pg dark cover"><div><div class="brand">{LOGO.replace("var(--accent)", "#c4f25a").replace("var(--bg)", "#0b0c0a")}loft</div>
<h1 class="serif">Your own crew of AI developers, <em>working from your Mac.</em></h1><p>{e(INTRO)}</p>
<div class="chips"><span>Runs on your Mac</span><span>Web app</span><span>iPhone app</span></div></div>
<div class="stage">{browser("web-chat-orchestrator")}{phone("overview")}</div></div>''']
    toc = "".join(f'<div><b>{i + 1:02d} · {e(eb)}</b><span>{e(t)}</span><p>{e(lead)}</p></div>' for i, (k, eb, t, lead, *_) in enumerate(SECTIONS))
    pages.append(f'''<div class="pg"><p class="eyebrow">Overview</p><h2 class="serif">What Loft does</h2>
<p class="lead" style="max-width:190mm">You give agents jobs. They work on your projects on their own and keep you in the loop. You decide what goes out. Here is everything it does today, in {len(SECTIONS)} parts.</p>
<div class="toc">{toc}</div><div class="foot"><span>Loft</span><span>2</span></div></div>''')
    for i, (key, eyebrow, title, lead, bullets, webs, phones) in enumerate(SECTIONS):
        shots = "".join(browser(w, f"b{j}") for j, w in enumerate(webs)) + "".join(phone(p, f"p{j}") for j, p in enumerate(phones))
        layout = "only-phones" if not webs else ("with-phone" if phones else "only-web")
        lis = "".join(f"<li>{e(b)}</li>" for b in bullets)
        pages.append(f'''<div class="pg"><div class="feat"><div><p class="eyebrow"><b>{i + 1:02d}</b>{e(eyebrow)}</p><h2 class="serif">{e(title)}</h2><p class="lead">{e(lead)}</p><ul>{lis}</ul></div>
<div class="shots {layout}">{shots}</div></div><div class="foot"><span>Loft · {e(eyebrow)}</span><span>{i + 3}</span></div></div>''')
    soon = "".join(f"<div><b>{e(t)}</b><span>{e(d)}</span></div>" for t, d in COMING_SOON)
    pages.append(f'''<div class="pg dark"><p class="eyebrow" style="color:#9aa18d">Coming soon</p><h2 class="serif" style="font-size:40pt">What's next</h2><div class="soon">{soon}</div>
<p class="serif" style="font-size:44pt;margin-top:22mm;line-height:1">Less typing. <em style="color:#c4f25a">More building.</em></p></div>''')
    return f"""<!doctype html><html><head><meta charset="utf-8"><title>Loft</title>
<style>{font_face()}{PRINT_CSS}</style></head><body>{''.join(pages)}</body></html>"""


(OUT / "index.html").write_text(page())
(OUT / "print.html").write_text(print_page())
print("wrote", OUT / "index.html", f"{(OUT / 'index.html').stat().st_size / 1e6:.1f} MB")
