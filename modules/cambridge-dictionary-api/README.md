# Cambridge Dictionary · 本机模块版 1.2.1-local.1

在 [Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api) 原项目基础上抽离查词模块，保留原 Cambridge 解析和字段。Node 进程直接请求 Cambridge，不经过别人部署的词典 API。

仅查询 Cambridge；`verbs: []` 为原成功 JSON 的兼容字段，不对应功能模块。原 MIT 许可证及作者署名保留，改动范围见 [UPSTREAM.md](UPSTREAM.md)。本次交付本机模块、CLI 和可选的本机 HTTP 测试入口。

## 安装依赖一次

需要 Node.js 22 或更新版本，本次在 Node 24.18.0 上验证。解压后在这个目录打开终端：

```bash
npm exec --yes --package=pnpm@9.15.9 -- pnpm install --frozen-lockfile
```

## 本机直接查询

```bash
node cli.js terminal
node cli.js "run on the spot" en-cn
node cli.js runtime en
```

CLI 直接调用本机模块，输出原格式 JSON，**不用启动 HTTP 服务**。默认词库为 `en-cn`，第二个参数可指定 `en / uk / en-cn / en-tw`。接口每次查询所指定词库；英汉缺词时由调用方选择英英，现有 WordSide 已有这条回退路径。`runtime` 的英英结果实际头词是 `run-time`，遵守 Cambridge 原返回。

## 软件集成入口

包的默认入口已经是 `lookup.js`，导入时不会启动服务器。模块调用：

```js
const { createLookup } = require('./modules/cambridge-dictionary-api');
async function main() {
  const lookup = createLookup();
  try {
    const entry = await lookup('en-cn', 'terminal');
    console.log(entry);
  } finally {
    lookup.close();
  }
}
main().catch(console.error);
```

上例从 WordSide 仓库根调用；在模块目录内可 `require('./lookup')`。在持续运行的软件中复用一个 lookup 实例，才能复用连接与内存缓存；每次 CLI 退出都会清理该次进程缓存。WordSide 0.1.0 起改用原生 Rust 直接查词；此 JS 模块用于独立查询、来源与兼容对照，桌面运行不依赖它。

## 模块结构

| 文件 | 职责 |
| --- | --- |
| `lookup.js` | 本机查词入口、完整结果组装、同词并发合并与缓存 |
| `dictionary/cambridge.js` | 原项目 Cambridge URL 规则、网页获取与原字段解析 |
| `lib/http-client.js` | 原 Axios 客户端与共享连接、大小/重定向限制 |
| `lib/cache.js` | 30 分钟内存缓存，最多 1000 条、32 MiB 序列化 JSON |
| `cli.js` | 不启动服务的本机直接查词 |
| `data.js` / `index.js` | 可选的原格式本机 HTTP 接口，供现有客户端测试 |

缓存不写入磁盘，关闭进程就清空。完整网页读完并通过结构校验后才返回一次结果，不输出半条释义。成功字段仍为 `word / pos / verbs / pronunciation / definition`；词性、译文、核心例句、音标、音频与可选 `source` 的提取方式保留原实现。

原项目没有解析的定义外“更多例句”仍不混入核心释义。`400` 为输入或词库错误，`404` 为缺词，`502` 为上游拒绝/连接失败/网页校验失败，`504` 为超时；网络错误不再统一伪装成“没有单词”。

## 缺词拼写建议

确认 `404 / WORD_NOT_FOUND` 后，模块会读取同词库的 Cambridge `spellcheck` 页面。若存在有效候选，抛出的 DictionaryError 附带 `suggestions: string[]`（最多 10 个）；可选 HTTP 接口仍返回 404，并在原 `{error, code}` 上增加这个可选字段。没有候选时不增加字段。CLI 的错误 JSON（stderr，退出码 1）也保留这个可选字段。成功词条的五个顶层字段保持不变。

例如查询 `termnal` 或 `terminalx`，2026-10-14 的真实 Cambridge 页面均建议 `terminal`。候选来自词典自己的列表，保持顺序；不会擅自选定候选、自动查询候选或把它当作原词的释义。调用方可显示按钮，用户点击后再次正常查询。WordSide 在英汉与英英均未命中时合并去重后显示。

普通命中只发一个请求。缺词才多发一个建议请求；建议等待上限为 3 秒，且共用首请求起算的 10 秒总预算。建议页超时、阻断、残缺或不可用时保留已经确认的缺词结果；原词条请求发生这些错误时仍返回 502/504，不请求建议。无结果页的 browse 范围与导航不会作为候选。

这是拼写建议，不能保证覆盖所有缩写、片段和错词。选出的片段若本身是有效词条，正常返回它。

## 可选：兼容旧 HTTP 客户端

原接口保留作独立测试入口：

```bash
npm start
```

仅监听 `127.0.0.1:3000`，Ctrl+C 退出；Windows 也可用 start-local.cmd。原测试页面为 `http://127.0.0.1:3000/`，接口示例为 `http://127.0.0.1:3000/api/dictionary/en-cn/terminal`。当前内置 WordSide 不使用这个服务或 api_base 配置，正常使用无需运行。

## 检查

```bash
npm test
npm run probe
```

`npm test` 是离线上游模拟、原代码契约及真实 loopback HTTP 测试，不依赖公共网络；`npm run probe` 会实际查询公开 Cambridge。`tools/verify-upstream.js` 可对保存源页执行原代码与拆分后解析器的逐字段比较。实际检查与测量见 [验收说明.md](验收说明.md)。

请求沿用 Axios 的 `HTTPS_PROXY / HTTP_PROXY / NO_PROXY` 环境变量。
