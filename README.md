# ncm 音频指纹生成 WASM Demo

将单声道 Float32 PCM 转为音频指纹或 AAC 音频。

## 接口

| 导出 | 采样率 | 返回值 |
| --- | --- | --- |
| `fingerprint_afp(samples, sampleRate)` | 8000 Hz | AFP 指纹的 Base64 字符串 |
| `fingerprint_hfp(samples, sampleRate)` | 16000 Hz | HFP 指纹的 Base64 字符串 |
| `audio_asr(samples, sampleRate)` | 16000 Hz | AAC-LC、48 kbps、单声道 MP4 的 Base64 字符串 |

三个函数均返回标准 Base64 字符串，可直接放入请求的 `fingerPrint`（AFP/HFP）或 `recordAudio`（ASR）字段。先完成 WASM 初始化；在 Web Worker 中处理音频，避免阻塞界面。ASR 音频不是 ADTS。AAC 使用 rusty_aac。

### ES Module

```js
import init, { fingerprint_afp } from './pkg/ncm_recognition.js'

await init()
// samples 是已重采样到 8000 Hz 的单声道 Float32Array。
const body = {
  duration: samples.length / 8000,
  fingerPrint: fingerprint_afp(samples, 8000),
}
```

### 普通浏览器脚本 / Worker

用 `--browser` 导出的绑定提供全局函数 `NcmRecognition`，不包含 Node require 或 ESM import/export。JS 与 WASM 是两个独立文件，不内嵌二进制。

下面是普通 Worker 的最小用法，两个产物与 Worker 放在同一目录：

```js
importScripts('./ncm_recognition.js')

self.onmessage = async ({ data: samples }) => {
  await NcmRecognition({ module_or_path: './ncm_recognition_bg.wasm' })
  self.postMessage(NcmRecognition.fingerprint_afp(samples, 8000))
}
```

页面用 `new Worker('./recognition-worker.js')` 加载，不设置 `type: 'module'`。普通页面也可通过 `<script src="./ncm_recognition.js"></script>` 加载绑定，初始化和三个函数的调用方式相同。

采样率不符或 PCM 包含 NaN/Infinity 时抛出错误。ASR 还要求输入非空、幅值位于 `[-1, 1]`；PCM16 可除以 32768 归一化。HFP 没有可用音高时返回空字符串 `""`，调用方不应发送空指纹。

## 构建与测试

需要 Rust/Cargo 和 Node.js 22+，从本目录执行：

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
cargo test --locked
node scripts/build.cjs
node scripts/build.cjs --browser
node --test tests/wasm.test.mjs
```

- `node scripts/build.cjs`：ES Module 产物写入 `pkg/`，适合 `import`。
- `node scripts/build.cjs --browser`：普通浏览器产物写入 `pkg/browser/`，适合 `script` / `importScripts`。
- 自定义目录：`node scripts/build.cjs <输出目录>` 或 `node scripts/build.cjs --browser <输出目录>`。

两种输出均包含 JS 绑定、独立 WASM 二进制和类型声明。例如浏览器输出为 [ncm_recognition.js](pkg/browser/ncm_recognition.js)、[ncm_recognition_bg.wasm](pkg/browser/ncm_recognition_bg.wasm) 和 [ncm_recognition.d.ts](pkg/browser/ncm_recognition.d.ts)。运行时只需前两个文件，放在同一目录；不要混用不同构建或不同模式的产物。JS 是可直接阅读的绑定代码，二进制单独存储。调用方不需要安装 Rust。

首次构建需要下载依赖；后续使用提交的 Cargo.lock。升级 wasm-bindgen 时同步升级 CLI。测试同时覆盖两种输出。

## 代码划分

- [lib.rs](src/lib.rs)：三个公共入口。
- [afp.rs](src/afp.rs)：频谱峰提取。
- [hfp.rs](src/hfp.rs)、[hfp_hmm.rs](src/hfp_hmm.rs)：音高候选、时序跟踪与量化。
- [fingerprint.rs](src/fingerprint.rs)：指纹序列化、压缩与加密。
- [asr.rs](src/asr.rs)、[mp4.rs](src/mp4.rs)：AAC 编码与 MP4 封装。
- [wasm.test.mjs](tests/wasm.test.mjs)：WASM 接口、输入边界及固定音频输出回归。

修改计算顺序、浮点精度、HMM 边界或封装字段时先跑回归测试，不要仅因代码更短而改写数学表达式。测试不需要网络或个人音频。构建产物由脚本生成，不手改 JS 绑定。

项目采用 [MIT 许可证](LICENSE)。
