import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import FeedDetailPage from './FeedDetailPage.vue'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('vue-router', () => ({
  useRoute: () => ({ params: { id: '74685504' } }),
  useRouter: () => ({ back: vi.fn() }),
}))

async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

describe('详情页验证重放', () => {
  beforeEach(() => invoke.mockReset())

  it('解包验证重放的 data，显示正文并重新加载评论', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'detail') {
        return {
          ok: false,
          data: null,
          error: {
            kind: 'captcha_needed',
            challenge: { id: 'abcdef0123456789', field: '' },
          },
        }
      }
      if (command === 'complete_captcha') {
        return {
          ok: true,
          data: {
            status: 1,
            data: { id: 74685504, username: '真实用户', message: '验证后的正文' },
          },
          error: null,
        }
      }
      if (command === 'replies') {
        return { ok: true, data: { data: [] }, error: null }
      }
      if (command === 'close_captcha_window') return undefined
      if (command === undefined) return undefined
      throw new Error(`unexpected command: ${String(command)}`)
    })

    const wrapper = mount(FeedDetailPage)
    await settle()
    expect(wrapper.text()).toContain('需要完成人机验证')
    const windowButton = wrapper.findAll('button').find((button) => button.text() === '打开验证窗口')
    await windowButton!.trigger('click')

    const done = wrapper.findAll('button').find((button) => button.text() === '我已完成验证')
    expect(done).toBeDefined()
    await done!.trigger('click')
    await settle()

    expect(wrapper.text()).toContain('真实用户')
    expect(wrapper.text()).toContain('验证后的正文')
    expect(wrapper.text()).toContain('评论')
    expect(invoke).toHaveBeenCalledWith('replies', { id: '74685504', page: 1 })
    expect(wrapper.text()).not.toContain('需要完成人机验证')
  })

  it('重放返回不可渲染的数据时保留错误和重试入口', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'detail') {
        return {
          ok: false,
          data: null,
          error: { kind: 'captcha_needed', challenge: { id: 'abcdef0123456789', field: '' } },
        }
      }
      if (command === 'complete_captcha') {
        return { ok: true, data: { status: 1, data: [] }, error: null }
      }
      if (command === undefined) return undefined
      throw new Error(`unexpected command: ${String(command)}`)
    })

    const wrapper = mount(FeedDetailPage)
    await settle()
    const done = wrapper.findAll('button').find((button) => button.text() === '我已完成验证')
    await done!.trigger('click')
    await settle()

    expect(wrapper.text()).toContain('服务器响应格式异常')
    expect(wrapper.findAll('button').some((button) => button.text() === '重新加载详情')).toBe(true)
  })
})
