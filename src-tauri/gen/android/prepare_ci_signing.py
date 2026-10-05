"""在受控 CI 环境生成签名文件；凭据只通过环境变量进入，不拼进 shell。"""

import base64
import binascii
import os
from pathlib import Path


def properties_value(value: str) -> str:
    """Java Properties 按 ISO-8859-1 读取；转义换行、反斜线和 Unicode，保留密码原值。"""
    out = []
    for char in value:
        escaped = {'\\': '\\\\', '\n': '\\n', '\r': '\\r', '\t': '\\t', '\f': '\\f',
                   ' ': '\\ ', '=': '\\=', ':': '\\:', '#': '\\#', '!': '\\!'}.get(char)
        if escaped is not None:
            out.append(escaped)
        elif 32 <= ord(char) < 127:
            out.append(char)
        else:
            units = char.encode('utf-16-be')
            out.extend('\\u' + units[n:n+2].hex() for n in range(0, len(units), 2))
    return ''.join(out)


def prepare(android_root: Path, environment: dict[str, str]) -> str:
    config = android_root / 'keystore.properties'
    if config.exists() or config.is_symlink():
        raise ValueError('已有签名配置，拒绝覆盖')
    encoded = environment.get('ANDROID_KEY_BASE64', '')
    alias = environment.get('ANDROID_KEY_ALIAS', '')
    password = environment.get('ANDROID_KEY_PASSWORD', '')
    if not all((encoded, alias, password)):
        if environment.get('GITHUB_EVENT_NAME') == 'workflow_dispatch':
            return 'debug'
        raise ValueError('发布标签缺少完整 Android 签名 Secrets')
    if len(encoded) > 65536 or not 1 <= len(alias) <= 256 or not 1 <= len(password) <= 4096:
        raise ValueError('Android 签名 Secrets 长度无效')
    if '\x00' in alias or '\x00' in password or '\n' in alias or '\r' in alias:
        raise ValueError('Android 签名 Secrets 格式无效')
    try:
        key = base64.b64decode(''.join(encoded.split()), validate=True)
    except (ValueError, binascii.Error):
        raise ValueError('Android keystore Base64 无效') from None
    if not key or len(key) > 49152 or not (key.startswith(b'\xfe\xed\xfe\xed') or key[0] == 0x30):
        raise ValueError('Android keystore 格式无效')
    temporary = Path(environment.get('RUNNER_TEMP', ''))
    if not temporary.is_absolute() or not temporary.is_dir():
        raise ValueError('CI 临时目录无效')
    private = temporary / 'sitzfleisch-android-signing'
    private.mkdir(mode=0o700)
    store = private / 'release.jks'
    with os.fdopen(os.open(store, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as output:
        output.write(key)
    text = '\n'.join(f'{name}={properties_value(value)}' for name, value in (
        ('keyAlias', alias), ('password', password), ('storeFile', str(store)),
    )) + '\n'
    with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w', encoding='ascii') as output:
        output.write(text)
    return 'release'


if __name__ == '__main__':
    try:
        mode = prepare(Path(__file__).resolve().parent, dict(os.environ))
        output = Path(os.environ['GITHUB_OUTPUT'])
        if not output.is_absolute():
            raise ValueError('CI 输出路径无效')
        with output.open('a', encoding='utf-8') as file:
            file.write(f'mode={mode}\n')
        print(f'Android 构建模式：{mode}')
    except (ValueError, OSError, KeyError):
        raise SystemExit('Android CI 签名配置失败；请核对 Secrets 与临时目录，凭据不会写入日志') from None
