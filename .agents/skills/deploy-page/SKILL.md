---
name: deploy-page
description: Generate a single-page interactive or static HTML page and deploy it — either to the project's GitHub Pages site (site/) or standalone to Cloudflare Pages.
triggers:
  intent_patterns:
    - "做个网页|做个页面|生成网页|生成页面|写个网页|写个页面|搭个页面"
    - "做个展示页|做个落地页|做个landing page|做个demo页"
    - "部署|deploy|publish|发布|上线|host|放到线上"
    - "给我一个链接|生成一个链接|我要分享|做个可以分享的"
    - "可视化|visualization|dashboard|仪表盘|数据看板"
    - "做个简历页|做个作品集|portfolio|resume page"
    - "互动页面|interactive|小游戏|mini game|小工具|tool page"
    - "对比页|对比表|comparison|pricing page|定价页"
    - "时间线|timeline|路线图|roadmap"
    - "邀请函|invitation|贺卡|greeting card|公告页|announcement"
    - "产品介绍|product page|feature page|功能介绍"
    - "表单|form|问卷|survey|投票|poll"
    - "帮我画个.*页面|帮我搞个.*网站|帮我弄个.*page"
    - "写个.*explainer|做个.*详解|做个.*讲解"
    - "论文.*页面|paper.*page|研究.*可视化"
  context_signals:
    keywords: ["网页", "页面", "HTML", "deploy", "部署", "上线", "landing", "展示",
               "可视化", "dashboard", "简历", "portfolio", "互动", "interactive",
               "分享", "链接", "demo", "发布", "visualization", "page", "explainer",
               "详解", "讲解"]
  confidence_threshold: 0.5
priority: 8
requires_tools: [bash, write]
max_tokens: 300
cooldown: 15
output:
  format: markdown
  artifacts: true
  artifact_type: document
---

# deploy-page

Generate a single-page HTML application and deploy it live. Two deployment targets:

1. **GitHub Pages** (default for explainer/docs pages) — write to `site/`, commit, push. Auto-deployed via GitHub Actions to `https://eliagent.github.io/<slug>.html`
2. **Cloudflare Pages** (standalone apps, tools, demos) — deploy via wrangler to `https://<project>.pages.dev`

## When to Trigger

This skill activates whenever the user wants a **standalone web page** they can open or share.

- **展示/汇报**: "做个项目介绍页", "把这个数据做成可视化页面"
- **技术详解**: "写个 PALU 论文的 explainer 页面", "做个架构讲解页"
- **Landing page**: "做个产品落地页", "做个活动页"
- **工具/互动**: "做个 JSON 格式化工具", "做个小计算器"
- **数据可视化**: "把这些数据画成图表页面", "做个 dashboard"
- **分享需求**: "做个可以分享的页面"
- **已有 HTML**: "帮我把这个 HTML 部署上去"

## Design System — Eli Dark Theme

All generated pages MUST use this design system for visual consistency with existing site pages. This is non-negotiable.

### CSS Variables (copy verbatim into every page)

```css
:root {
  --bg: #07111f;
  --bg2: #0c1830;
  --panel: rgba(13, 24, 42, 0.84);
  --panel-2: rgba(16, 31, 54, 0.96);
  --text: #ebf3ff;
  --muted: #9eb1cd;
  --line: rgba(135, 170, 255, 0.18);
  --blue: #76c8ff;
  --cyan: #6ef0e8;
  --green: #8cffb6;
  --yellow: #ffd76e;
  --red: #ff8d8d;
  --shadow: 0 24px 80px rgba(0, 0, 0, 0.34);
  --radius: 24px;
  --max: 1180px;
}
```

### Required Base Styles

```css
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  font-family: Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  color: var(--text);
  background:
    radial-gradient(circle at top left, rgba(70, 143, 255, 0.16), transparent 30%),
    radial-gradient(circle at top right, rgba(72, 228, 219, 0.14), transparent 28%),
    linear-gradient(180deg, var(--bg) 0%, #091426 46%, #07101d 100%);
  line-height: 1.68;
}
a { color: inherit; }
.wrap { width: min(var(--max), calc(100vw - 32px)); margin: 0 auto; }
```

### Component Library (use as needed)

**Hero section:**
```css
.hero { padding: 72px 0 28px; border-bottom: 1px solid rgba(255,255,255,0.06); }
.pill {
  display: inline-flex; align-items: center; gap: 10px;
  padding: 8px 14px; border-radius: 999px; font-size: 13px;
  color: var(--cyan); background: rgba(110, 240, 232, 0.09);
  border: 1px solid rgba(110, 240, 232, 0.2);
}
h1 { font-size: clamp(38px, 7vw, 74px); letter-spacing: -0.04em; max-width: 980px; margin: 0 0 14px; line-height: 1.15; }
h2 { font-size: clamp(28px, 4vw, 42px); letter-spacing: -0.03em; margin: 0 0 14px; line-height: 1.15; }
.lead { max-width: 980px; font-size: 18px; color: var(--muted); }
```

**Cards:**
```css
.card {
  background: linear-gradient(180deg, rgba(255,255,255,0.03), rgba(255,255,255,0.015));
  border: 1px solid var(--line); border-radius: var(--radius);
  padding: 22px; box-shadow: var(--shadow); backdrop-filter: blur(12px);
}
```

**Grid layouts:**
```css
.grid-2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 18px; }
.grid-3 { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 18px; }
.grid-4 { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 18px; }
@media (max-width: 980px) { .grid-2, .grid-3, .grid-4 { grid-template-columns: 1fr; } }
```

**Metrics:**
```css
.metric { font-size: clamp(28px, 5vw, 46px); font-weight: 800; color: var(--blue); letter-spacing: -0.04em; }
```

**Sticky nav:**
```css
nav {
  position: sticky; top: 0; z-index: 20;
  backdrop-filter: blur(18px); background: rgba(7, 17, 31, 0.78);
  border-bottom: 1px solid rgba(255,255,255,0.06);
}
nav .wrap { display: flex; gap: 14px; overflow-x: auto; white-space: nowrap; padding: 14px 0; scrollbar-width: none; }
nav a {
  text-decoration: none; color: var(--muted); font-size: 14px;
  padding: 8px 12px; border-radius: 999px; border: 1px solid transparent;
}
nav a:hover { color: var(--text); border-color: var(--line); background: rgba(255,255,255,0.03); }
```

**Flow steps:**
```css
.flow { display: grid; gap: 14px; margin-top: 10px; }
.step {
  border-left: 3px solid var(--blue); padding: 10px 0 10px 16px;
  background: linear-gradient(90deg, rgba(118, 200, 255, 0.08), transparent 70%);
  border-radius: 0 14px 14px 0;
}
```

**Tables:**
```css
.compare {
  width: 100%; border-collapse: collapse; overflow: hidden;
  border-radius: 18px; border: 1px solid var(--line); background: rgba(8, 16, 29, 0.55);
}
.compare th, .compare td { padding: 14px 16px; text-align: left; border-bottom: 1px solid rgba(255,255,255,0.06); }
.compare th { color: var(--text); background: rgba(255,255,255,0.03); }
```

**Status badges:**
```css
.badge { display: inline-block; padding: 4px 10px; border-radius: 999px; font-size: 12px; font-weight: 700; }
.ok { background: rgba(140,255,182,.12); color: var(--green); }
.warn { background: rgba(255,215,110,.12); color: var(--yellow); }
.risk { background: rgba(255,141,141,.12); color: var(--red); }
```

**Notes/callouts:**
```css
.note {
  padding: 16px 18px; border-radius: 16px;
  border: 1px solid rgba(255,215,110,.18); background: rgba(255,215,110,.07); color: #ffe7aa;
}
```

**Code blocks:**
```css
.equation {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 15px; color: #d9e8ff; background: rgba(5, 10, 18, 0.65);
  white-space: pre-wrap; border: 1px solid rgba(122, 165, 255, 0.18);
  border-radius: 16px; padding: 16px; overflow-x: auto; margin: 0;
}
```

### Content Guidelines

- **Language:** Match the user's language. Chinese input → Chinese page. English input → English page.
- **Title:** Big, punchy `<h1>` with a `.pill` tag above it for category/topic.
- **Lead paragraph:** One sentence that captures the core insight. Use `<strong>` for key phrases.
- **Metrics:** Use `.metric` cards for key numbers. 2-4 metrics in the hero area.
- **Sections:** Each major topic gets a `<section>` with `<h2>` and a sticky nav link.
- **Tables:** Use `.compare` for comparison tables. Use `.badge` for status indicators.
- **Flow:** Use `.step` elements for sequential processes or timelines.
- **Footer:** Simple `.footer` with date and source attribution.

### HTML Template Structure

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{TITLE}</title>
  <meta name="description" content="{DESCRIPTION}" />
  <style>
    /* CSS variables + base styles + components (from above) */
  </style>
</head>
<body>
  <header class="hero">
    <div class="wrap">
      <span class="pill">{CATEGORY}</span>
      <h1>{TITLE}</h1>
      <p class="lead">{LEAD}</p>
      <div class="hero-grid"><!-- metrics + summary cards --></div>
    </div>
  </header>
  <nav><div class="wrap"><!-- section links --></div></nav>
  <!-- sections -->
  <footer class="footer"><div class="wrap">{DATE} · {ATTRIBUTION}</div></footer>
</body>
</html>
```

## Workflow

### Step 0: Decide deployment target

- If the page is an **explainer, docs, or project-related page** → **GitHub Pages** (write to `site/`)
- If the page is a **standalone tool, demo, or external project** → **Cloudflare Pages** (via wrangler)
- If unclear, default to GitHub Pages.

### Step 1: Generate the HTML

Use the Design System above. The page MUST:
- Be self-contained (all CSS inline, JS inline except CDN libs)
- Be responsive (mobile breakpoint at 980px)
- Use the Eli Dark Theme CSS variables
- Follow the HTML template structure

**Recommended CDN libraries** (use only when needed):
- Charts: `<script src="https://cdn.jsdelivr.net/npm/chart.js"></script>`
- Icons: Lucide via CDN
- Syntax highlighting: Prism.js via CDN
- 3D/WebGL: Three.js via CDN
- Maps: Leaflet via CDN
- Math: KaTeX via CDN

### Step 2A: Deploy to GitHub Pages (default)

```bash
# Generate a kebab-case filename from the title
SLUG="<kebab-case-title>"
SITE_DIR="site"

# Write the HTML
# (use Write tool to create site/${SLUG}.html)

# Commit and push — GitHub Actions auto-deploys
git add "site/${SLUG}.html"
git commit -m "feat: add ${SLUG} explainer page"
git push
```

The page will be live at `https://eliagent.github.io/${SLUG}.html` after GitHub Actions completes (~1 min).

**Important:** The `site/` directory has a GitHub Actions workflow (`.github/workflows/pages.yml`) that auto-deploys on push to `main` when `site/**` changes. Just commit and push — deployment is automatic.

### Step 2B: Deploy to Cloudflare Pages (standalone)

```bash
DEPLOY_DIR=$(mktemp -d)
# Write index.html to $DEPLOY_DIR
bash $SKILL_DIR/deploy.sh "$DEPLOY_DIR" "<project-name>"
rm -rf "$DEPLOY_DIR"
```

The page will be live at `https://<project-name>.pages.dev`.

### Step 3: Present the URL

Show the user the live URL prominently:
- GitHub Pages: `https://eliagent.github.io/<slug>.html`
- Cloudflare Pages: `https://<project-name>.pages.dev`

## Constraints

- GitHub Pages: push to `site/` on `main` branch triggers auto-deploy
- Cloudflare Pages: requires `wrangler` installed and authenticated
- Max file size: 25 MiB per asset
- All pages must use the Eli Dark Theme design system
- No build step — everything is vanilla HTML/CSS/JS
