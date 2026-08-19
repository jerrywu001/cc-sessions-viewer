import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import {
  fileIconFor,
  IconFileDoc,
  IconFileSheet,
  IconFileSlides,
  IconFileImage,
  IconFileVideo,
  IconFileAudio,
  IconFileArchive,
  IconFileCode,
  IconJson,
  IconFile,
  IconGrok,
  agentIcons,
} from '../../src/components/icons'

describe('fileIconFor', () => {
  it.each([
    ['report.docx', IconFileDoc],
    ['notes.md', IconFileDoc],
    ['a.pdf', IconFileDoc],
    ['plain.txt', IconFileDoc],
    ['data.xlsx', IconFileSheet],
    ['table.csv', IconFileSheet],
    ['deck.pptx', IconFileSlides],
    ['pic.png', IconFileImage],
    ['favicon.svg', IconFileImage],
    ['clip.mp4', IconFileVideo],
    ['song.mp3', IconFileAudio],
    ['bundle.zip', IconFileArchive],
    ['archive.tar.gz', IconFileArchive],
    ['main.rs', IconFileCode],
    ['App.vue', IconFileCode],
    ['config.yaml', IconFileCode],
    ['data.json', IconJson],
  ])('maps %s to its type icon', (path, expected) => {
    expect(fileIconFor(path)).toBe(expected)
  })

  it('is case-insensitive on the extension', () => {
    expect(fileIconFor('PHOTO.JPG')).toBe(IconFileImage)
    expect(fileIconFor('REPORT.DOCX')).toBe(IconFileDoc)
  })

  it('resolves by the basename extension, ignoring directories', () => {
    expect(fileIconFor('/Users/me/My.Docs/report.xlsx')).toBe(IconFileSheet)
  })

  it('falls back to the generic file icon', () => {
    expect(fileIconFor('Makefile')).toBe(IconFile)
    expect(fileIconFor('archive')).toBe(IconFile)
    expect(fileIconFor('.gitignore')).toBe(IconFile) // 无后缀点文件不算扩展名
    expect(fileIconFor('weird.qqzz')).toBe(IconFile) // 未知扩展名
    expect(fileIconFor('/a/b/c/')).toBe(IconFile) // 目录路径
  })
})

describe('agent icons', () => {
  it('registers a dedicated Grok Build icon', () => {
    expect(agentIcons.grok).toBe(IconGrok)
    const wrapper = mount(IconGrok)
    expect(wrapper.find('img').attributes('src')).toBe('/grok-icon.png')
  })
})
