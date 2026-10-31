import { describe, expect, it } from 'vitest'
import { extractValidate } from './captcha'

describe('extractValidate', () => {
  it('从完整 cookie 串里取出 validate', () => {
    const raw = 'uid=42; username=bob; validate=abc%2Fdef; SESSID=xyz'
    expect(extractValidate(raw)).toBe('abc%2Fdef')
  })

  it('单独一段 validate=xxx 也能取到', () => {
    expect(extractValidate('validate=hello123')).toBe('hello123')
  })

  it('没有前缀时把整段当作 validate', () => {
    expect(extractValidate('  token123  ')).toBe('token123')
  })

  it('去掉内部空白', () => {
    expect(extractValidate('a b\tc')).toBe('abc')
  })

  it('空输入返回空串', () => {
    expect(extractValidate('')).toBe('')
    expect(extractValidate('   ')).toBe('')
  })

  it('URL 查询串形式不会吞掉后续参数', () => {
    // 监督者指出的缺陷：早期正则会把 &foo=1 一起吞进去
    expect(extractValidate('?validate=abc123&foo=1')).toBe('abc123')
  })

  it('问号起始也能识别', () => {
    expect(extractValidate('?validate=xyz')).toBe('xyz')
  })

  it('井号锚点会截断', () => {
    expect(extractValidate('validate=abc#frag')).toBe('abc')
  })

  it('Cookie 串中间的分号形式', () => {
    expect(extractValidate('uid=1; validate=abc; path=/')).toBe('abc')
  })
})
