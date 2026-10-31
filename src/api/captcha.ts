import { invoke } from '@tauri-apps/api/core'
import { ApiError, type BackendError } from './index'

/**
 * 人机验证窗口。
 *
 * 设计约束（见 docs/PROTOCOL.md §9）：
 * - 验证必须在**真实 WebView** 里完成，桌面端无法用脚本绕过
 * - 凭证（validate）只在内存中流转，不写日志、不进 URL、不落盘
 * - 每个挑战只允许重放一次
 */

export interface CaptchaRequest {
  challengeId: string
  field: string
  /** 用户完成验证后拿到的原始 validate 字符串 */
  validate: string
}

interface InvokeResult<T> {
  ok: boolean
  data: T | null
  error: BackendError | null
}

/** 打开酷安验证页的地址。 */
export function captchaHelpUrl(): string {
  return 'https://account.coolapk.com/auth/login?type=mobile'
}

/**
 * 提交验证结果。
 *
 * 后端会**立刻用凭证重放原请求**，返回重放得到的响应体。
 * 调用方应直接用返回值渲染，而不是再发一次同样的请求——
 * 那样会白白消耗一次重放机会。
 */
export async function submitCaptcha(
  req: CaptchaRequest,
): Promise<Record<string, unknown>> {
  const res = await invoke<InvokeResult<Record<string, unknown>>>(
    'submit_validation',
    {
      challengeId: req.challengeId,
      field: req.field,
      validate: req.validate,
    },
  )
  if (!res.ok) {
    throw new ApiError(
      res.error ?? { kind: 'parse', message: '验证提交失败' },
    )
  }
  return res.data ?? {}
}

/**
 * 从用户粘贴的文本里提取 validate。
 *
 * 用户可能整段复制 Cookie 串、URL 查询串，或只复制值本身。
 * 这里做宽松但安全的提取：
 *
 * - `?validate=xxx&foo=1` / `; validate=xxx; path=/` → 取 xxx
 * - `validate=xxx` 单独出现 → 取 xxx
 * - 没有任何前缀 → 把整段（去空白）当作 validate
 *
 * 终止符要排除 `&` 和 `#`，否则会把后续参数一起吞进来。
 */
export function extractValidate(text: string): string {
  const trimmed = text.trim()
  if (!trimmed) return ''
  const m = /(?:^|[;?\s])validate=([^;&\s#]+)/i.exec(trimmed)
  if (m) return m[1]
  return trimmed.replace(/\s+/g, '')
}
