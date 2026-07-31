import { describe, expect, it } from 'vitest'

import { getErrorMessage } from './error'

describe('getErrorMessage', () => {
  it('preserves a raw string rejected by a Tauri command', () => {
    expect(getErrorMessage('配置校验失败：network_name 不能为空', '无法保存档案。')).toBe(
      '配置校验失败：network_name 不能为空',
    )
  })

  it('reads Error and common structured rejection shapes', () => {
    expect(getErrorMessage(new Error('服务拒绝写入档案'), '无法保存档案。')).toBe('服务拒绝写入档案')
    expect(getErrorMessage({ message: 'Core 配置校验未通过' }, '无法保存档案。')).toBe(
      'Core 配置校验未通过',
    )
    expect(getErrorMessage({ error: { message: '服务拒绝写入档案' } }, '无法保存档案。')).toBe(
      '服务拒绝写入档案',
    )
  })

  it('uses the fallback for empty or unreadable rejections', () => {
    const cyclic: { cause?: unknown } = {}
    cyclic.cause = cyclic

    expect(getErrorMessage(new Error(''), '无法保存档案。')).toBe('无法保存档案。')
    expect(getErrorMessage({ message: '' }, '无法保存档案。')).toBe('无法保存档案。')
    expect(getErrorMessage(cyclic, '无法保存档案。')).toBe('无法保存档案。')
  })
})
