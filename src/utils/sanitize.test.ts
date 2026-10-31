import { describe, expect, it } from 'vitest'
import { sanitizeHtml } from './sanitize'

/**
 * sanitizer 的对抗性测试。
 *
 * 为什么要单独立一组测试：
 * 详情页的公开页兜底用 v-html 渲染外部 UGC HTML，
 * 而渲染环境是持有 Tauri invoke 的 WebView。
 * 一旦清洗有漏洞，注入的脚本可以越权调用后端命令
 * （例如改写持久化的设备身份），风险是应用级的。
 */
describe('sanitizeHtml 对抗性', () => {
  it('剥掉 script 标签', () => {
    const out = sanitizeHtml('<p>hi</p><script>alert(1)</script>')
    expect(out).not.toContain('<script')
    expect(out).toContain('<p>hi</p>')
  })

  it('剥掉内联事件处理器', () => {
    const out = sanitizeHtml('<img src="x" onerror="alert(1)">')
    expect(out.toLowerCase()).not.toContain('onerror')
  })

  it('剥掉 onclick 等事件属性', () => {
    const out = sanitizeHtml('<a href="#" onclick="evil()">x</a>')
    expect(out.toLowerCase()).not.toContain('onclick')
  })

  it('剥掉 javascript: 协议', () => {
    const out = sanitizeHtml('<a href="javascript:alert(1)">x</a>')
    expect(out.toLowerCase()).not.toContain('javascript:')
  })

  it('剥掉大小写混淆的 javascript 协议', () => {
    const out = sanitizeHtml('<a href="JaVaScRiPt:alert(1)">x</a>')
    expect(out.toLowerCase()).not.toContain('javascript:')
  })

  it('剥掉 iframe', () => {
    const out = sanitizeHtml('<iframe src="https://evil"></iframe>')
    expect(out.toLowerCase()).not.toContain('<iframe')
  })

  it('剥掉 object 与 embed', () => {
    const out = sanitizeHtml('<object data="x"></object><embed src="y">')
    expect(out.toLowerCase()).not.toContain('<object')
    expect(out.toLowerCase()).not.toContain('<embed')
  })

  it('剥掉 form 与 input', () => {
    const out = sanitizeHtml('<form><input name="p"></form>')
    expect(out.toLowerCase()).not.toContain('<form')
    expect(out.toLowerCase()).not.toContain('<input')
  })

  it('剥掉 style 标签与内联样式', () => {
    const out = sanitizeHtml('<style>body{display:none}</style><p style="color:red">x</p>')
    expect(out.toLowerCase()).not.toContain('<style')
    expect(out.toLowerCase()).not.toContain('style=')
  })

  it('剥掉 svg（防止 SVG 解析差异绕过）', () => {
    const out = sanitizeHtml('<svg><script>alert(1)</script></svg>')
    expect(out.toLowerCase()).not.toContain('<svg')
    expect(out.toLowerCase()).not.toContain('<script')
  })

  it('保留正常图文结构', () => {
    const input = '<p>正文</p><img src="https://img.coolapk.com/a.jpg" alt="图">'
    const out = sanitizeHtml(input)
    expect(out).toContain('<p>正文</p>')
    expect(out).toContain('https://img.coolapk.com/a.jpg')
  })

  it('保留代码块与表格', () => {
    const input = '<pre><code>x=1</code></pre><table><tr><td>a</td></tr></table>'
    const out = sanitizeHtml(input)
    expect(out).toContain('<pre>')
    expect(out).toContain('<code>')
    expect(out).toContain('<table>')
  })

  it('外链加上 noopener noreferrer', () => {
    const out = sanitizeHtml('<a href="https://example.com">x</a>')
    expect(out).toContain('noopener')
    expect(out).toContain('noreferrer')
  })

  it('空输入返回空串', () => {
    expect(sanitizeHtml('')).toBe('')
  })

  it('嵌套的 script 也被剥掉', () => {
    const out = sanitizeHtml('<div><div><script>alert(1)</script></div></div>')
    expect(out.toLowerCase()).not.toContain('<script')
  })

  it('data: 开头的脚本协议被剥', () => {
    const out = sanitizeHtml('<a href="data:text/html,<script>alert(1)</script>">x</a>')
    expect(out.toLowerCase()).not.toContain('data:text/html')
  })
})
