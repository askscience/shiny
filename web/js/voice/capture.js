/** Clamp microphone samples to signed PCM16 before uploading. */
export function floatTo16BitPCM(float32) {
  const out = new Int16Array(float32.length);
  for (let i = 0; i < float32.length; i++) {
    const s = Math.max(-1, Math.min(1, float32[i]));
    out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
  }
  return out;
}


/** Permission prompts may finish after the listening gesture was cancelled.
 * Keep the grant local until its generation is confirmed; a stale prompt must
 * never overwrite or stop a newer listening session's stream.
 */
export async function acquireMicrophone(mediaDevices, constraints, isCurrent) {
  if (!isCurrent()) return null;
  let stream;
  try { stream = await mediaDevices.getUserMedia(constraints); }
  catch (error) {
    if (!isCurrent()) return null;
    throw error;
  }
  if (!isCurrent()) {
    stream.getTracks().forEach((track) => track.stop());
    return null;
  }
  return stream;
}
