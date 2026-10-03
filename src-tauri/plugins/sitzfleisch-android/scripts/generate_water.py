"""仅用标准库生成可重复的 0.2 秒喝水提示音；用户可替换生成的 WAV。"""

from pathlib import Path
import math
import struct
import wave


def main():
    output = Path(__file__).resolve().parents[3] / 'gen/android/app/src/main/res/raw/water.wav'
    output.parent.mkdir(parents=True, exist_ok=True)
    rate = 44100
    frames = 8820
    samples = bytearray()
    for index in range(frames):
        seconds = index / rate
        envelope = math.sin(math.pi * index / (frames - 1)) ** 2
        tone = 0.75 * math.sin(2 * math.pi * 587.33 * seconds)
        tone += 0.25 * math.sin(2 * math.pi * 783.99 * seconds)
        samples.extend(struct.pack('<h', round(0.28 * 32767 * envelope * tone)))
    with wave.open(str(output), 'wb') as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(rate)
        audio.writeframes(samples)
    print(f'已生成 {output}：单声道、16 位 PCM、44100 Hz、0.2 秒')


if __name__ == '__main__':
    main()
