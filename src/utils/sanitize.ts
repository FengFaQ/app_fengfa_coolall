import DOMPurify from 'dompurify'

/**
 * 清洗从 www.coolapk.com 公开分享页抓回的 HTML。
 *
 * 为什么必须清洗：公开页内容是用户生成的（UGC）。
 * 渲染环境是持有 `__TAURI_INTERNALS__.invoke` 的 Tauri WebView，
 * 一旦注入脚本成功，攻击面就不是「网页级」而是「能调用后端命令的应用级」——
 * 例如改写持久化的设备身份、读取本地配置。
 *
 * 策略：白名单式放行，只保留正文需要的标签与属性。
 */
const ALLOWED_TAGS = [
  'a',
  'p',
  'br',
  'span',
  'div',
  'strong',
  'b',
  'em',
  'i',
  'u',
  's',
  'blockquote',
  'pre',
  'code',
  'ul',
  'ol',
  'li',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'table',
  'thead',
  'tbody',
  'tr',
  'th',
  'td',
  'img',
  'figure',
  'figcaption',
  'hr',
]

const ALLOWED_ATTR = ['href', 'src', 'alt', 'title', 'width', 'height', 'class']

let hookInstalled = false

function installHook() {
  if (hookInstalled) return
  hookInstalled = true
  // 外链一律加 noopener noreferrer，防止 window.opener 反向控制
  DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (node.tagName === 'A' && node.getAttribute('href')) {
      node.setAttribute('target', '_blank')
      node.setAttribute('rel', 'noopener noreferrer')
    }
  })
}

export function sanitizeHtml(html: string): string {
  if (!html) return ''
  installHook()
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS,
    ALLOWED_ATTR,
    // 显式禁止可执行协议与内联事件
    ALLOW_DATA_ATTR: false,
    FORBID_TAGS: ['script', 'style', 'iframe', 'object', 'embed', 'form', 'input'],
    FORBID_ATTR: ['onerror', 'onload', 'onclick', 'style'],
  })
}
