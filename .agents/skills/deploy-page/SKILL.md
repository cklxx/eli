---
name: deploy-page
description: Generate a single-page interactive or static HTML page and deploy it to Cloudflare Pages, returning a live URL.
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
  context_signals:
    keywords: ["网页", "页面", "HTML", "deploy", "部署", "上线", "landing", "展示",
               "可视化", "dashboard", "简历", "portfolio", "互动", "interactive",
               "分享", "链接", "demo", "发布", "visualization", "page"]
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

Generate a single-page HTML application and deploy it live to Cloudflare Pages. One skill: describe what you want → get a URL.

## When to Trigger

This skill activates whenever the user wants a **standalone web page** they can open or share. Common scenarios:

- **展示/汇报**: "做个项目介绍页", "把这个数据做成可视化页面"
- **Landing page**: "做个产品落地页", "做个活动页"
- **工具/互动**: "做个 JSON 格式化工具", "做个小计算器", "做个投票页"
- **个人页面**: "做个简历页", "做个作品集", "做个个人主页"
- **信息展示**: "做个对比表", "做个时间线", "做个路线图页面"
- **社交/趣味**: "做个邀请函", "做个贺卡", "做个小游戏"
- **数据可视化**: "把这些数据画成图表页面", "做个 dashboard"
- **分享需求**: "我需要一个链接发给别人看", "做个可以分享的页面"
- **已有 HTML**: "帮我把这个 HTML 部署上去", "这个页面放到线上"

## Workflow

### Step 1: Generate the HTML

If the user provides HTML content or a file, use it directly. Otherwise, **generate a complete single-file HTML page**.

**Generation principles:**
- **Self-contained**: all CSS and JS inline, no external dependencies except CDN libraries
- **Responsive**: mobile-first, works on all screen sizes
- **Polished**: use modern CSS (grid, flexbox, variables, transitions), not bare HTML
- **Fast**: no heavy frameworks unless needed; vanilla JS + lightweight CDN libs preferred
- **Beautiful defaults**: pick a cohesive color palette, good typography (Inter/system fonts), proper spacing

**Recommended CDN libraries** (use only when needed):
- Charts: `<script src="https://cdn.jsdelivr.net/npm/chart.js"></script>`
- Icons: `<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/lucide-static/font/lucide.min.css">`
- Animations: `<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/animate.css">`
- Markdown rendering: `<script src="https://cdn.jsdelivr.net/npm/marked/marked.min.js"></script>`
- Syntax highlighting: Prism.js or Highlight.js via CDN
- 3D/WebGL: Three.js via CDN
- Maps: Leaflet via CDN

### Step 2: Write to deploy directory

```bash
DEPLOY_DIR=$(mktemp -d)
```

Use the Write tool to create `$DEPLOY_DIR/index.html`. If there are additional assets, put them in `$DEPLOY_DIR` too.

### Step 3: Deploy

Use the automated deploy script which handles project creation and deployment:

```bash
bash $SKILL_DIR/deploy.sh "$DEPLOY_DIR" "<project-name>"
```

The script:
1. Validates `index.html` exists in the deploy directory
2. Checks `wrangler` is installed
3. Creates the Cloudflare Pages project if it doesn't exist (idempotent)
4. Deploys with `wrangler pages deploy`
5. Extracts and prints both the preview URL and production URL

**Project naming rules:**
- Derive from the page's purpose: `team-dashboard`, `project-timeline`, `ckl-resume`
- kebab-case, lowercase, no special characters
- If user specifies a name, use it
- Redeploying with the same name updates the existing site

### Step 4: Clean up

```bash
rm -rf "$DEPLOY_DIR"
```

### Step 5: Present the URL

Show the user the live URL prominently. The production URL is always `https://<project-name>.pages.dev`.

## Constraints

- **wrangler** must be installed and authenticated (`wrangler login`)
- Max file size: 25 MiB per asset
- Project names must be globally unique on Cloudflare Pages
- If project name conflicts, append a short random suffix and retry
- Preview deploys: add `--branch=<name>` for non-production versions
