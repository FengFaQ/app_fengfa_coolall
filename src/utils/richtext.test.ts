import { describe, it, expect } from 'vitest'
import { renderRichText } from './richtext'

describe('renderRichText 表情与图片解析', () => {
  it('命中映射的 [表情] 替换为本地 emoji 图片', () => {
    const out = renderRichText('今天天气[哈哈哈]不错')
    expect(out).toContain('<img class="ck-emoji" src="/emojis/coolapk_emotion_1_hahaha.png"')
    expect(out).toContain('alt="[哈哈哈]"')
  })

  it('未命中的 [token] 保留原文', () => {
    const out = renderRichText('这是[不存在的表情]测试')
    expect(out).toContain('[不存在的表情]')
    expect(out).not.toContain('<img')
  })

  it('[图片] 渲染为图片占位图标', () => {
    const out = renderRichText('看图[图片]呀')
    expect(out).toContain('class="ck-photo"')
    expect(out).toContain('📷')
  })

  it('[置顶] 等文字徽标渲染为 badge', () => {
    const out = renderRichText('[置顶]公告')
    expect(out).toContain('class="ck-badge"')
    expect(out).toContain('置顶')
  })

  it('内嵌远程图片（image.coolapk.com）保留 src', () => {
    const out = renderRichText('<img src="https://image.coolapk.com/a.jpg" alt="图">')
    expect(out).toContain('src="https://image.coolapk.com/a.jpg"')
  })

  it('远程图片上的 onerror 等事件被剥离', () => {
    const out = renderRichText('<img src="https://image.coolapk.com/a.jpg" onerror="alert(1)">')
    expect(out).not.toContain('onerror')
  })

  it('emoji 图片在清洗后仍存在（未被误删）', () => {
    const out = renderRichText('[微笑]')
    expect(out).toContain('/emojis/')
    expect(out).toContain('<img')
  })

  it('脚本标签被剥离', () => {
    const out = renderRichText('<p>hi</p><script>alert(1)</script>')
    expect(out).not.toContain('<script')
    expect(out).toContain('hi')
  })

  it('换行被转换为 <br>', () => {
    const out = renderRichText('第一行\n第二行')
    expect(out).toContain('<br>')
  })

  it('空字符串返回空', () => {
    expect(renderRichText('')).toBe('')
  })
})
