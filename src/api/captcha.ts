import { invoke } from '@tauri-apps/api/core'
import { ApiError, type BackendError } from './index'

/**
 * 人机验证。
 *
 * # 为什么用内嵌窗口而不是外部浏览器
 *
 * 外部浏览器是独立进程，它的 Cookie 归浏览器管，应用读不到。
 * 用户手动复制粘贴既不可靠，凭证还会经过剪贴板。
 *
 * 这里用 Tauri 的 `WebviewWindow` 在应用内打开酷安验证页，
 * 用户完成后由后端调用 `WebviewWindow::cookies()` 直接读取凭证。
 *
 * # 凭证流向
 *
 * WebView Cookie → Rust 内存 → 拼装 proof → 重放原请求
 * 全程不落盘、不进日志、不经过前端。
 */

interface InvokeResult<T> {
  ok: boolean
  data: T | null
  error: BackendError | null
}

/** 读取验证窗口 Cookie 的结果。 */
export interface CookieReadResult {
  validate: string | null
  cookie_names: string[]
  message: string
}

/** 打开应用内嵌验证窗口。若已打开则聚焦。 */
export async function openCaptchaWindow(): Promise<void> {
  await invoke<void>('open_captcha_window')
}

/** 关闭验证窗口。 */
export async function closeCaptchaWindow(): Promise<void> {
  await invoke<void>('close_captcha_window')
}

/**
 * 读取验证窗口里的凭证（仅用于诊断，不会拿到前端）。
 *
 * 正常流程不需要调用它——`completeCaptcha` 内部会读。
 */
export async function readCaptchaValidate(): Promise<CookieReadResult> {
  return invoke<CookieReadResult>('read_captcha_validate')
}

/**
 * 用户确认已完成验证：后端读取凭证、重放原请求、返回结果。
 *
 * 返回值就是重放得到的响应体，调用方应直接渲染，
 * 不要再发一次同样的请求——那会白白消耗重放机会。
 */
export async function completeCaptcha(): Promise<Record<string, unknown>> {
  const res = await invoke<InvokeResult<Record<string, unknown>>>(
    'complete_captcha',
  )
  if (!res.ok) {
    throw new ApiError(res.error ?? { kind: 'parse', message: '验证未通过' })
  }
  return res.data ?? {}
}
