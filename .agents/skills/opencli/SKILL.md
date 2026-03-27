---
name: opencli
description: "Make any website your CLI: 355+ commands across 61 sites — browse, search, post, scrape, automate."
triggers:
  intent_patterns:
    - "浏览器|browser|网页|webpage|打开网站|open.*url|截图|screenshot"
    - "bilibili|b站|微博|weibo|知乎|zhihu|小红书|xiaohongshu|雪球|xueqiu"
    - "youtube|twitter|x\\.com|bloomberg|bbc|arxiv|wikipedia"
    - "boss直聘|boss|招聘|recruit"
    - "微信|weixin|公众号|weread|读书"
    - "podcast|播客|小宇宙"
    - "stock|股票|行情|quote|options|期权"
    - "scrape|抓取|爬|extract|download|下载"
    - "抖音|douyin|豆瓣|douban|即刻|jike|v2ex"
    - "twitter|推特|reddit|instagram|tiktok|facebook"
    - "hackernews|hacker.news|hn|stackoverflow"
    - "google|谷歌|medium|substack|notion"
    - "京东|jd|携程|ctrip|什么值得买|smzdm"
    - "huggingface|hf|devto|lobsters"
  context_signals:
    keywords: ["浏览器", "browser", "网页", "截图", "打开", "navigate", "搜索", "热门", "feed", "热搜"]
  confidence_threshold: 0.6
priority: 6
requires_tools: [bash]
max_tokens: 4000
cooldown: 10
---

# opencli

Make any website your CLI. 355+ commands across 61 sites. Zero setup. AI-powered.

## Usage

```bash
opencli <site> <command> [options]
```

All commands support `-f json|table|yaml|md|csv` for output format. Use `-v` for debug output.

## Discovery

```bash
opencli list                    # all available commands
opencli list --site bilibili    # commands for a specific site
opencli <site> <cmd> --help     # help for a specific command
```

## Key Sites & Commands

### Web & Search
| Command | Description |
|---------|-------------|
| `web read --url <url>` | Fetch any page as Markdown [cookie] |
| `google search --query <q>` | Google search |
| `google news` | Google News headlines |
| `google trends` | Daily trending searches |
| `wikipedia search --q <query>` | Search Wikipedia |
| `wikipedia summary --title <t>` | Article summary |
| `arxiv search --query <q>` | Search arXiv papers |
| `arxiv paper --id <id>` | Paper details |

### Social — Chinese
| Command | Description |
|---------|-------------|
| `bilibili search --keyword <q>` | Search Bilibili |
| `bilibili hot` | Trending videos |
| `bilibili subtitle --bvid <id>` | Video subtitles |
| `bilibili feed` | 关注的人的动态 |
| `bilibili history` | 观看历史 |
| `bilibili download --bvid <id>` | 下载视频 (需 yt-dlp) |
| `weibo hot` | 微博热搜 |
| `weibo search --keyword <q>` | Search Weibo |
| `weibo feed` | 首页时间线 |
| `zhihu hot` | 知乎热榜 |
| `zhihu search --keyword <q>` | Search Zhihu |
| `zhihu question --url <url>` | 问题详情和回答 |
| `xiaohongshu search --keyword <q>` | Search Xiaohongshu |
| `xiaohongshu feed` | 首页推荐 |
| `xiaohongshu publish ...` | 发布图文笔记 |
| `xiaohongshu creator-stats` | 创作者数据总览 |
| `douyin videos` | 作品列表 |
| `douyin stats` | 作品数据分析 |
| `douyin publish ...` | 定时发布视频 |
| `douban movie-hot` | 豆瓣电影热门 |
| `douban search --keyword <q>` | 搜索电影/图书/音乐 |
| `douban top250` | 电影 Top250 |
| `jike feed` | 即刻首页动态 |
| `jike search --keyword <q>` | 搜索帖子 |
| `v2ex hot` | V2EX 热门话题 |
| `v2ex latest` | 最新话题 |

### Social — Global
| Command | Description |
|---------|-------------|
| `twitter search --query <q>` | Search tweets [intercept] |
| `twitter timeline` | Home timeline [cookie] |
| `twitter post --text <t>` | Post tweet [ui] |
| `twitter reply --url <url> --text <t>` | Reply to tweet [ui] |
| `twitter profile --username <u>` | User profile [cookie] |
| `twitter trending` | Trending topics [cookie] |
| `twitter bookmarks` | Your bookmarks [cookie] |
| `reddit hot` | Reddit hot posts |
| `reddit search --query <q>` | Search Reddit |
| `reddit subreddit --name <n>` | Subreddit posts |
| `reddit read --url <url>` | Post + comments |
| `instagram search --query <q>` | Search users |
| `instagram profile --username <u>` | User profile |
| `instagram user --username <u>` | User's recent posts |
| `tiktok search --query <q>` | Search videos |
| `tiktok explore` | Trending videos |
| `tiktok profile --username <u>` | User profile |
| `facebook feed` | News feed |
| `facebook search --query <q>` | Search |
| `youtube search --query <q>` | Search YouTube |
| `youtube transcript --url <url>` | Video transcript |
| `youtube video --url <url>` | Video metadata |
| `youtube channel --url <url>` | Channel info + videos |

### Developer
| Command | Description |
|---------|-------------|
| `hackernews top` | HN top stories |
| `hackernews search --query <q>` | Search HN |
| `hackernews show` | Show HN posts |
| `stackoverflow search --query <q>` | Search SO questions |
| `stackoverflow hot` | Hot questions |
| `devto top` | Top DEV.to articles |
| `lobsters hot` | Lobsters hot stories |
| `hf top` | Top HuggingFace papers |
| `linux-do hot` | linux.do 热门 |

### Content & Reading
| Command | Description |
|---------|-------------|
| `medium search --query <q>` | Search Medium |
| `medium feed` | 热门文章 |
| `substack search --query <q>` | Search Substack |
| `weread search --query <q>` | 微信读书搜索 |
| `weread shelf` | 我的书架 |
| `weread highlights --bookId <id>` | 书中划线 |
| `weixin download --url <url>` | 公众号文章→Markdown |

### Finance
| Command | Description |
|---------|-------------|
| `xueqiu stock --symbol <s>` | Stock quote |
| `xueqiu hot-stock` | Hot stocks |
| `xueqiu watchlist` | 自选股列表 |
| `xueqiu feed` | 首页时间线 |
| `sinafinance news` | 新浪财经 7x24 快讯 |
| `barchart quote --symbol <s>` | US stock quote |
| `barchart options --symbol <s>` | Options chain |
| `barchart flow` | Unusual options activity |
| `yahoo-finance quote --symbol <s>` | Yahoo Finance quote |

### News
| Command | Description |
|---------|-------------|
| `bbc news` | BBC headlines |
| `bloomberg main` | Bloomberg top stories |
| `bloomberg markets` | Markets news |
| `bloomberg tech` | Tech news |
| `reuters search --query <q>` | Reuters 搜索 |

### Recruitment
| Command | Description |
|---------|-------------|
| `boss recommend` | View recommended candidates |
| `boss greet --id <id>` | Send greeting to candidate |
| `boss batchgreet` | 批量招呼 |
| `boss resume --id <id>` | 查看简历 |
| `boss chatlist` | 聊天列表 |
| `boss search --query <q>` | Search jobs |
| `boss stats` | 职位数据统计 |

### E-commerce & Travel
| Command | Description |
|---------|-------------|
| `jd item --url <url>` | 京东商品详情 |
| `ctrip search --keyword <q>` | 携程搜索 |
| `smzdm search --keyword <q>` | 什么值得买搜索 |

### Podcasts
| Command | Description |
|---------|-------------|
| `apple-podcasts search --q <q>` | Search podcasts |
| `apple-podcasts top` | Top podcasts chart |
| `xiaoyuzhou podcast --id <id>` | Podcast profile |
| `xiaoyuzhou episode --id <id>` | Episode details |

### AI & Tools
| Command | Description |
|---------|-------------|
| `yollomi generate --prompt <p>` | AI image generation |
| `yollomi video --prompt <p>` | AI video generation |
| `yollomi remove-bg --image <url>` | Remove background |
| `yollomi edit --prompt <p> --image <url>` | AI image editing |
| `yollomi upscale --image <url>` | Upscale resolution |
| `jimeng generate --prompt <p>` | 即梦 AI 文生图 |

### Advanced
```bash
opencli explore <url>            # Discover site APIs & strategies
opencli record <url>             # Record browser API calls → YAML
opencli generate <url>           # One-shot: explore → synthesize → register
opencli cascade <url>            # Find simplest working strategy
opencli doctor                   # Check browser bridge connectivity
```

## Strategy Types

Commands use different strategies indicated by `[tag]`:
- **public** — No auth needed, direct API/RSS
- **cookie** — Uses browser cookies (must be logged in to the site in Chrome)
- **ui** — Browser UI automation
- **intercept** — Intercepts browser network requests

## Typical Workflows

**Read a web page:**
```bash
opencli web read --url https://example.com -f md
```

**Research a topic:**
```bash
opencli arxiv search --query "transformer attention" -f json
opencli wikipedia summary --title "Attention (machine learning)"
opencli hackernews search --query "transformer" -f json
```

**Check markets:**
```bash
opencli xueqiu stock --symbol SH000001
opencli barchart flow -f json
opencli sinafinance news
```

**Browse Chinese social:**
```bash
opencli weibo hot
opencli zhihu hot
opencli bilibili hot
opencli douyin videos
opencli xiaohongshu feed
```

**Browse global social:**
```bash
opencli twitter trending
opencli reddit hot
opencli hackernews top
opencli tiktok explore
```
