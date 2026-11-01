import emojiMap from './emoji-map.json'
import { sanitizeHtml } from './sanitize'

const EMOJI_REGEX = /\[[^\]\n]+\]/g

/**
 * 把酷安消息文本转换为可安全渲染的富文本 HTML。
 *
 * 对齐参考项目 c001apk（Flutter 电脑版）`html_text.dart` 的策略：
 *   1. 换行转 `<br/>`
 *   2. `[表情]` token 命中映射时替换为本地 emoji 图片 / 占位图标 / 文字徽标
 *   3. 内嵌的远程图片（`<img src="https://...">`）原样保留
 *   4. 末了统一过一遍 `sanitizeHtml`，杜绝注入脚本
 *
 * 注意：本函数接收的是来自网络的未信任文本，**必须**始终以 `sanitizeHtml` 收尾。
 */
export function renderRichText(raw: string): string {
  if (!raw) return ''
  let html = raw.replace(/\r\n?/g, '\n').replace(/\n/g, '<br/>')
  html = html.replace(EMOJI_REGEX, (token) => {
    const value = (emojiMap as Record<string, string>)[token]
    if (!value) return token // 未命中：保留原文（兜底，与参考项目一致）
    if (value.endsWith('.png')) {
      return `<img class="ck-emoji" src="/emojis/${value}" alt="${token}" />`
    }
    if (value.endsWith('.icon')) {
      // 图片占位（如 [图片]）：用内联图标表示，不依赖外部资源
      return '<span class="ck-photo" title="图片" aria-label="图片">📷</span>'
    }
    // 纯文字徽标（[置顶] / [楼主] / [层主] 等）
    return `<span class="ck-badge">${value}</span>`
  })
  return sanitizeHtml(html)
}
