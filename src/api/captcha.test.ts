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
})
