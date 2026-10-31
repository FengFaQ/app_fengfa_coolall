import { invoke } from '@tauri-apps/api/core'

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

/**
 * 打开验证页面。
 *
 * 当前实现使用系统浏览器打开酷安登录页让用户完成验证，
 * 然后把拿到的 validate 字符串粘回来。
 *
 * 后续会替换为 Tauri 独立窗口 + 内嵌 WebView 自动回传。
 */
export function captchaHelpUrl(): string {
  return 'https://account.coolapk.com/auth/login?type=mobile'
}

/** 提交验证结果给后端。challenge_id 不匹配时后端会拒绝。 */
export async function submitCaptcha(req: CaptchaRequest): Promise<boolean> {
  return invoke<boolean>('submit_validation', {
    challengeId: req.challengeId,
    field: req.field,
    validate: req.validate,
  })
}

/**
 * 从用户粘贴的文本里提取 validate。
 *
 * 用户可能整段复制，这里做宽松提取：
 * - 优先取 `validate=xxx` 形式
 * - 否则把整段当作 validate（去掉空白）
 */
export function extractValidate(text: string): string {
  const trimmed = text.trim()
  if (!trimmed) return ''
  const m = /(?:^|[;\s])validate=([^;\s]+)/i.exec(trimmed)
  if (m) return m[1]
  return trimmed.replace(/\s+/g, '')
}
