import { invoke } from '@tauri-apps/api/core'
import { ApiError, unwrapFeedDetail, type BackendError } from './index'

/**
 * 人机验证。
 *
 * # 真实机制（从参考项目 c001apk 核实）
 *
 * **不是打开酷安的验证页**，而是在应用内构造一个本地页面：
 *
 * ```
 * 1. 页面引入 https://cstaticdun.126.net/load.min.js（网易易盾）
 * 2. 调 initNECaptcha({ captchaId, onVerify })
 * 3. onVerify 回调拿到 data.validate
 * 4. 通过 Tauri IPC 回传给 Rust（命令 captcha_result）
 * 5. 用户点确认后，Rust 用凭证重放原请求
 * ```
 *
 * 早期我打开 `account.coolapk.com/auth/login` 是错的——
 * 那个页面根本不会把 validate 交给应用。
 *
 * # 凭证流向
 *
 * 易盾 JS → Tauri IPC → Rust 暂存 → 拼装 proof → 重放原请求
 * 全程不落盘、不进日志、不经过前端渲染。
 */

interface InvokeResult<T> {
  ok: boolean
  data: T | null
  error: BackendError | null
}

/**
 * 打开验证窗口。
 *
 * 后端会取当前挑战的 captchaId 构造页面，所以必须先有挑战
 * （即已经遇到过 1004）。
 */
export async function openCaptchaWindow(): Promise<void> {
  await invoke<void>('open_captcha_window')
}

/** 关闭验证窗口。 */
export async function closeCaptchaWindow(): Promise<void> {
  await invoke<void>('close_captcha_window')
}

/**
 * 用户确认完成验证：后端用已收到的 validate 重放原请求。
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
  return unwrapFeedDetail(res.data ?? {})
}
