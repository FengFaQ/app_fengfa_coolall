// 从参考项目 TG-Twilight/c001apk 的 Flutter 实现生成表情映射与本地资源。
//
// 参考：apps/flutter/lib/utils/emoji_util.dart 的 EmojiUtil.emojiMap
// 该映射把酷安文本里的 [表情] token 对应到 assets/emojis 下的 PNG 文件名，
// 与 c001apk 电脑版（Flutter 桌面端）的渲染策略一致。
//
// 产物：
//   - src/utils/emoji-map.json   { "[哈哈哈]": "coolapk_emotion_1_hahaha.png", ... }
//   - public/emojis/*.png        本地表情/酷币图片（被打包进 Tauri 应用）

import { readFileSync, writeFileSync, mkdirSync, copyFileSync, readdirSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const dartPath = join(root, '_upstream/c001apk-twilight/apps/flutter/lib/utils/emoji_util.dart')
const dart = readFileSync(dartPath, 'utf8')

const map = {}
const re = /'(\[[^\]\n]*\]|[^'\n]*)'\s*:\s*'([^'\n]*)'/g
let m
while ((m = re.exec(dart))) {
  const key = m[1]
  const val = m[2]
  if (key === 'class' || key.startsWith('EmojiUtil')) continue
  map[key] = val
}

const jsonPath = join(root, 'src/utils/emoji-map.json')
writeFileSync(jsonPath, JSON.stringify(map, null, 2) + '\n', 'utf8')

const srcDir = join(root, '_upstream/c001apk-twilight/apps/flutter/assets/emojis')
const outDir = join(root, 'public/emojis')
mkdirSync(outDir, { recursive: true })
let copied = 0
for (const f of readdirSync(srcDir)) {
  if (f.endsWith('.png')) {
    copyFileSync(join(srcDir, f), join(outDir, f))
    copied++
  }
}

// 校验：映射里声明的 png 是否都能在资源目录找到
const missing = Object.values(map)
  .filter((v) => v.endsWith('.png') && !readdirSync(outDir).includes(v))
console.log(`mapped ${Object.keys(map).length} tokens; copied ${copied} png; missing ${missing.length}`)
if (missing.length) console.log('missing:', missing.slice(0, 20))
