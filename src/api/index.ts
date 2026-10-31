import { invoke } from '@tauri-apps/api/core'

/// 后端统一返回结构，对齐 lib.rs 的 InvokeResult
export interface InvokeResult<T> {
  ok: boolean
  data: T | null
  error: BackendError | null
}

export interface CaptchaChallenge {
  id: string
  field: string
}

export type BackendError =
  | { kind: 'network'; message: string }
  | { kind: 'http'; status: number }
  | { kind: 'business'; status: number; message: string }
  | { kind: 'captcha_needed'; challenge: CaptchaChallenge }
  | { kind: 'auth_required'; message: string }
  | { kind: 'rate_limited'; message: string }
  | { kind: 'client_upgrade'; message: string }
  | { kind: 'risk_control'; message: string }
  | { kind: 'parse'; message: string }

export interface SessionInfo {
  challengeId: string | null
  challengeField: string | null
  szlmId: string
  userAgent: string
}

/// 业务异常：把后端错误包装成可读消息
export class ApiError extends Error {
  readonly detail: BackendError

  constructor(detail: BackendError) {
    super(errorMessage(detail))
    this.name = 'ApiError'
    this.detail = detail
  }

  get retryable(): boolean {
    return ['network', 'http', 'rate_limited'].includes(this.detail.kind)
  }

  get needsCaptcha(): boolean {
    return this.detail.kind === 'captcha_needed'
  }
}

export function errorMessage(detail: BackendError): string {
  switch (detail.kind) {
    case 'network':
      return '连接失败，请检查网络后重试'
    case 'http':
      return `请求失败（HTTP ${detail.status}）`
    case 'captcha_needed':
      return '请求需要人机验证'
    case 'auth_required':
      return '登录状态已失效，请重新登录'
    case 'rate_limited':
      return '请求过于频繁，请稍后重试'
    case 'client_upgrade':
      return '当前客户端协议未被接受，请更新客户端'
    case 'risk_control':
      return '服务端检测到网络环境异常'
    case 'parse':
      return '服务器响应格式异常'
    default:
      return (detail as { message?: string }).message ?? '未知错误'
  }
}

async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const res = await invoke<InvokeResult<T>>(cmd, args)
  if (!res.ok) {
    throw new ApiError(
      res.error ?? { kind: 'parse', message: '后端未返回错误详情' },
    )
  }
  return res.data as T
}

export interface FeedPage {
  data?: Array<Record<string, unknown>>
  [key: string]: unknown
}

/// 浏览目标：home 首页 / feed 动态
export function browse(target: 'home' | 'feed', page = 1) {
  return call<FeedPage>('browse', { target, page })
}

export function detail(id: string) {
  return call<Record<string, unknown>>('detail', { id })
}

export function replies(id: string, page = 1) {
  return call<FeedPage>('replies', { id, page })
}

export function search(keyword: string, page = 1) {
  return call<FeedPage>('search', { keyword, page })
}

export function sessionInfo() {
  return call<SessionInfo>('session_info')
}

export function submitValidation(
  challengeId: string,
  field: string,
  validate: string,
) {
  return invoke<boolean>('submit_validation', { challengeId, field, validate })
}

export function importShuzlmId(text: string) {
  return invoke<void>('import_shuzlm_id', { text })
}

/**
 * 设置是否允许公开页兜底。
 *
 * 只应在用户**明确放弃**人机验证时调用 true，随后重新请求详情。
 * 不要一遇到验证码就自动降级——那样用户永远无法通过验证。
 */
export function setPublicFallback(allow: boolean) {
  return invoke<void>('set_public_fallback', { allow })
}
