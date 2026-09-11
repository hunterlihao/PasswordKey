#!/usr/bin/env node
/**
 * 把 Vite 构建产物打包成「单文件 HTML」，供 Rust 侧用 include_str! 内嵌进 exe。
 *
 * 用法：
 *   node scripts/build-frontend.mjs            # 直接内联 ../frontend/dist
 *   node scripts/build-frontend.mjs --build    # 先执行 npm run build 再内联
 *
 * 产物：assets/app.html
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(here, '..');
const frontendDir = resolve(projectRoot, '../frontend');
const distDir = join(frontendDir, 'dist');
const outputFile = join(projectRoot, 'assets', 'app.html');

function runViteBuild() {
  const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
  console.log('[frontend] npm run build ->', frontendDir);
  execFileSync(npm, ['run', 'build'], { cwd: frontendDir, stdio: 'inherit', shell: process.platform === 'win32' });
}

/** 读取 dist 中的资源文件内容 */
function readAsset(relPath) {
  const clean = relPath.replace(/^\.?\//, '');
  const full = join(distDir, clean);
  if (!existsSync(full)) throw new Error(`资源不存在: ${full}`);
  return readFileSync(full, 'utf8');
}

function inline() {
  const indexPath = join(distDir, 'index.html');
  if (!existsSync(indexPath)) throw new Error(`未找到前端构建产物: ${indexPath}\n请先运行 npm run build`);

  let html = readFileSync(indexPath, 'utf8');
  const stats = { css: 0, js: 0, cssBytes: 0, jsBytes: 0 };

  // 1) 内联样式表
  html = html.replace(/<link\b[^>]*rel="stylesheet"[^>]*>/gi, (tag) => {
    const m = tag.match(/href="([^"]+)"/i);
    if (!m) return tag;
    const css = readAsset(m[1]);
    stats.css += 1;
    stats.cssBytes += Buffer.byteLength(css);
    return `<style>\n${css.replace(/<\/style/gi, '<\\/style')}\n</style>`;
  });

  // 2) 内联脚本（含 type="module"）
  html = html.replace(/<script\b[^>]*src="([^"]+)"[^>]*><\/script>/gi, (_tag, src) => {
    const js = readAsset(src);
    stats.js += 1;
    stats.jsBytes += Buffer.byteLength(js);
    const typeAttr = /type="module"/i.test(_tag) ? ' type="module"' : '';
    return `<script${typeAttr}>\n${js.replace(/<\/script/gi, '<\\/script')}\n</script>`;
  });

  // 3) 兜底校验：不允许再残留任何外部引用
  const leftover = html.match(/(?:src|href)="(?!data:|https?:|#)([^"]+)"/gi) || [];
  if (leftover.length) {
    throw new Error(`内联后仍有外部引用: ${leftover.join(', ')}`);
  }

  // 4) 便携版：注入一个标记，便于排查版本
  html = html.replace('</head>', `<!-- built by build-frontend.mjs: css=${stats.css} js=${stats.js} -->\n</head>`);

  mkdirSync(dirname(outputFile), { recursive: true });
  writeFileSync(outputFile, html, 'utf8');
  const kb = (Buffer.byteLength(html) / 1024).toFixed(1);
  console.log(
    `[frontend] 已内联 ${stats.css} 个样式(${(stats.cssBytes / 1024).toFixed(1)}KB) + ` +
      `${stats.js} 个脚本(${(stats.jsBytes / 1024).toFixed(1)}KB) -> ${outputFile} (${kb}KB)`,
  );
}

if (process.argv.includes('--build') || !existsSync(distDir)) runViteBuild();
inline();
