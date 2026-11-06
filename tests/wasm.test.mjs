import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import assert from 'node:assert/strict'
import test from 'node:test'

const source = readFileSync(
  new URL('../pkg/ncm_recognition.js', import.meta.url),
).toString('base64')
const web = await import('data:text/javascript;base64,' + source)
await web.default({
  module_or_path: readFileSync(
    new URL('../pkg/ncm_recognition_bg.wasm', import.meta.url),
  ),
})

const browserSource = readFileSync(
  new URL('../pkg/browser/ncm_recognition.js', import.meta.url),
  'utf8',
)
const browser = new Function(browserSource + '\nreturn NcmRecognition;')()
await browser({
  module_or_path: readFileSync(
    new URL('../pkg/browser/ncm_recognition_bg.wasm', import.meta.url),
  ),
})
const variants = [
  ['web', web],
  ['browser', browser],
]

const expected = {
  'fingerprint_afp/silence': {
    length: 80,
    sha256: 'a2c778caebc99f20b095dc6e95b86222cf24b0fb0011580e5be31f0bf7ddf0c1',
  },
  'fingerprint_afp/sine': {
    length: 160,
    sha256: '295ab85cbab88004cc8d34b7c428d113477ec4de14a414271e390dff61564a04',
  },
  'fingerprint_afp/melody': {
    length: 208,
    sha256: '3cf6116d39328524c99e543cc4f89e55817258cdf40b4e79b1543f58caae07d6',
  },
  'fingerprint_afp/noise': {
    length: 1856,
    sha256: 'b374a107c9b0816c678cf193f373def53ed23f3f1414044cc904af7c7f979807',
  },
  'fingerprint_afp/short': {
    length: 80,
    sha256: '3406564b8955b23a897530808cea752d7184024b8e7e9cbdcae9565d752a7969',
  },
  'fingerprint_hfp/silence': {
    length: 0,
    sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  },
  'fingerprint_hfp/sine': {
    length: 663,
    sha256: 'e1ce3d0e55d1a9ac3c89b4cb203d47954a1029ec7822ff4f2b45a15f17cb3822',
  },
  'fingerprint_hfp/melody': {
    length: 667,
    sha256: '4d0e9dfdf60a2d20ecfc6844ace63e95fa7cacaac182ed5ba8e85a805ccfe905',
  },
  'fingerprint_hfp/noise': {
    length: 0,
    sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  },
  'fingerprint_hfp/short': {
    length: 0,
    sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  },
  'audio_asr/silence': {
    length: 976,
    sha256: '6f84e02581e10fe1fcece9e5342975dcfbb5712576f409142a680e7e8b93c10b',
  },
  'audio_asr/sine': {
    length: 18147,
    sha256: '09ee7087a75e9f104bb56408e9210bec0a7a02f4fe9e3c29505ddf87836ddb24',
  },
  'audio_asr/melody': {
    length: 18342,
    sha256: '0a973184c7d61971f2d023f7c8ca23fd0e957f3fd0c4dfdd92cbfab5b78f2810',
  },
  'audio_asr/noise': {
    length: 18366,
    sha256: '3857792ba3d468e4f333cda263cd6c6e1778a39e6b9b73287f6a181cfe2cd5e3',
  },
  'audio_asr/short': {
    length: 1297,
    sha256: 'b87c8ce570e73ee01f41893efcd5e4bd239abbd78bf16c78014302646a5b8f64',
  },
}

function makeSamples(kind, rate) {
  const samples = new Float32Array(kind === 'short' ? 17 : rate * 3 + 37)
  let random = 0x12345678
  for (let i = 0; i < samples.length; i++) {
    const time = i / rate
    if (kind === 'sine') {
      samples[i] =
        Math.round(0.15 * Math.sin(2 * Math.PI * 440 * time) * 32767) / 32768
    } else if (kind === 'melody') {
      const frequency = [220, 261.63, 329.63, 293.66, 196][
        Math.floor(time / 0.4) % 5
      ]
      const phase = 2 * Math.PI * frequency * time
      const envelope = 0.5 - 0.5 * Math.cos((2 * Math.PI * (time % 0.4)) / 0.4)
      samples[i] =
        0.3 * envelope * (Math.sin(phase) + 0.25 * Math.sin(2 * phase))
    } else if (kind === 'noise') {
      random = (Math.imul(random, 1664525) + 1013904223) >>> 0
      samples[i] = ((random / 0x100000000) * 2 - 1) * 0.1
    } else if (kind === 'short') {
      samples[i] = i % 2 ? 0.01 : -0.01
    }
  }
  return samples
}

const cases = ['silence', 'sine', 'melody', 'noise', 'short']
const methods = [
  ['fingerprint_afp', 8000],
  ['fingerprint_hfp', 16000],
  ['audio_asr', 16000],
]

for (const [format, wasm] of variants) {
  for (const [method, rate] of methods) {
    for (const kind of cases) {
      test(format + '/' + method + '/' + kind, () => {
        const payload = wasm[method](makeSamples(kind, rate), rate)
        assert.equal(typeof payload, 'string')
        const bytes = Buffer.from(payload, 'base64')
        assert.equal(payload, bytes.toString('base64'))
        assert.deepEqual(
          {
            length: bytes.length,
            sha256: createHash('sha256').update(bytes).digest('hex'),
          },
          expected[method + '/' + kind],
        )
      })
    }
    test(format + '/' + method + '/invalid input', () => {
      assert.throws(() => wasm[method](new Float32Array([0]), 44100))
      for (const value of [NaN, Infinity, -Infinity]) {
        assert.throws(() => wasm[method](new Float32Array([value]), rate))
      }
    })
  }

  test(format + '/empty and normalized PCM boundaries', () => {
    assert.ok(wasm.fingerprint_afp(new Float32Array(), 8000).length > 0)
    assert.equal(wasm.fingerprint_hfp(new Float32Array(), 16000), '')
    assert.throws(() => wasm.audio_asr(new Float32Array(), 16000))
    assert.throws(() => wasm.audio_asr(new Float32Array([1.01]), 16000))
    assert.doesNotThrow(() => wasm.audio_asr(new Float32Array([-1, 1]), 16000))
  })

  test(format + '/public API and initialization helpers', () => {
    const names = [
      'audio_asr',
      'fingerprint_afp',
      'fingerprint_hfp',
      'initSync',
    ]
    if (format === 'web') names.push('default')
    assert.deepEqual(Object.keys(wasm).sort(), names.sort())
  })
}
