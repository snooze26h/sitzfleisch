// 请求与响应分别使用不同的 HMAC 消息格式，防止把客户端证明伪装成服务端证明。
export const PAIRING_KEY = "sitzfleisch.pairing.v1";
const encoder = new TextEncoder();
const validHex = (value) => typeof value === "string" && /^[0-9a-f]{64}$/.test(value);

export function validPairingCode(value) { return validHex(value); }
function bytes(value) {
  if (!validHex(value)) throw new Error("authentication-error");
  return Uint8Array.from(value.match(/../g), (pair) => Number.parseInt(pair, 16));
}
async function key(code) {
  return crypto.subtle.importKey("raw", bytes(code), { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
}

export function sessionChallenge() {
  const nonce = Array.from(crypto.getRandomValues(new Uint8Array(32)), (byte) => byte.toString(16).padStart(2, "0")).join("");
  return { nonce, headers: { "X-Sitzfleisch-Nonce": nonce } };
}

export async function requestAuthentication(code, applied, timestamp, session) {
  if (!validHex(session)) throw new Error("authentication-error");
  const { nonce } = sessionChallenge();
  const time = String(Math.floor(timestamp / 1000));
  const message = `sitzfleisch-request-v1\nGET /v1/rules\n2\n${applied ?? ""}\n${time}\n${nonce}\n${session}`;
  const proof = await crypto.subtle.sign("HMAC", await key(code), encoder.encode(message));
  return {
    nonce,
    headers: {
      "X-Sitzfleisch-Nonce": nonce,
      "X-Sitzfleisch-Time": time,
      "X-Sitzfleisch-Session": session,
      "X-Sitzfleisch-Proof": Array.from(new Uint8Array(proof), (byte) => byte.toString(16).padStart(2, "0")).join(""),
    },
  };
}

export async function verifyResponse(response, text, code, nonce) {
  const status = response.status === 200 ? "200 OK" : response.status === 503 ? "503 Service Unavailable" : null;
  const proof = response.headers.get("x-sitzfleisch-proof");
  if (!status || !validHex(proof)) throw new Error("authentication-error");
  const message = `sitzfleisch-response-v1\n${nonce}\n${status}\n${text}`;
  if (!await crypto.subtle.verify("HMAC", await key(code), bytes(proof), encoder.encode(message))) {
    throw new Error("authentication-error");
  }
}
