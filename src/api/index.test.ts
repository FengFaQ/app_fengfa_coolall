import { describe, expect, it, vi } from 'vitest'
import { ApiError, errorMessage } from '../api'

// 隔离 Tauri 的 invoke，测试纯逻辑部分
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

describe('errorMessage', () => {
  it('把网络错误映射为可读提示', () => {
    expect(errorMessage({ kind: 'network', message: 'x' })).toBe(
      '连接失败，请检查网络后重试',
    )
  })

  it('HTTP 错误带上状态码', () => {
    expect(errorMessage({ kind: 'http', status: 403 })).toBe(
      '请求失败（HTTP 403）',
    )
  })

  it('验证码错误有独立提示', () => {
    expect(
      errorMessage({
        kind: 'captcha_needed',
        challenge: { id: 'a', field: 'b' },
      }),
    ).toBe('请求需要人机验证')
  })

  it('业务错误透传服务端消息', () => {
    expect(
      errorMessage({ kind: 'business', status: -2, message: '应用不存在' }),
    ).toBe('应用不存在')
  })
})

describe('ApiError', () => {
  it('可重试的错误类型正确', () => {
    expect(new ApiError({ kind: 'network', message: 'x' }).retryable).toBe(true)
    expect(new ApiError({ kind: 'http', status: 500 }).retryable).toBe(true)
    expect(new ApiError({ kind: 'rate_limited', message: 'x' }).retryable).toBe(
      true,
    )
  })

  it('不可重试的错误类型正确', () => {
    expect(new ApiError({ kind: 'auth_required', message: 'x' }).retryable).toBe(
      false,
    )
    expect(
      new ApiError({ kind: 'client_upgrade', message: 'x' }).retryable,
    ).toBe(false)
  })

  it('能识别需要验证码', () => {
    const e = new ApiError({
      kind: 'captcha_needed',
      challenge: { id: 'abc', field: '_v2_post_token' },
    })
    expect(e.needsCaptcha).toBe(true)
  })

  it('保留原始错误详情', () => {
    const detail = { kind: 'http' as const, status: 403 }
    expect(new ApiError(detail).detail).toEqual(detail)
  })
})
