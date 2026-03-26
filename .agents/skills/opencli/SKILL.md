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

### Web & Reading
| Command | Description |
|---------|-------------|
| `web read --url <url>` | Fetch any page as Markdown |
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
| `weibo hot` | 微博热搜 |
| `weibo search --keyword <q>` | Search Weibo |
| `zhihu hot` | 知乎热榜 |
| `zhihu search --keyword <q>` | Search Zhihu |
| `xiaohongshu search --keyword <q>` | Search Xiaohongshu |
| `xiaohongshu publish ...` | Publish note |

### Social — Global
| Command | Description |
|---------|-------------|
| `youtube search --query <q>` | Search YouTube |
| `youtube transcript --url <url>` | Video transcript |
| `youtube video --url <url>` | Video metadata |

### Finance
| Command | Description |
|---------|-------------|
| `xueqiu stock --symbol <s>` | Stock quote |
| `xueqiu hot-stock` | Hot stocks |
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

### Recruitment
| Command | Description |
|---------|-------------|
| `boss recommend` | View recommended candidates |
| `boss greet --id <id>` | Send greeting to candidate |
| `boss search --query <q>` | Search jobs |

### Podcasts
| Command | Description |
|---------|-------------|
| `apple-podcasts search --q <q>` | Search podcasts |
| `xiaoyuzhou podcast --id <id>` | Podcast profile |

### AI & Tools
| Command | Description |
|---------|-------------|
| `yollomi generate --prompt <p>` | AI image generation |
| `yollomi video --prompt <p>` | AI video generation |
| `yollomi remove-bg --image <url>` | Remove background |

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
```

**Check markets:**
```bash
opencli xueqiu stock --symbol SH000001
opencli barchart flow -f json
```

**Browse Chinese social:**
```bash
opencli weibo hot
opencli zhihu hot
opencli bilibili hot
```
