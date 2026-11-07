// Turn the screencast frames into the README video.
// Frame durations come from the CDP timestamps; each `fast` span (the model
// thinking) is squeezed to FAST_SECONDS. Output: MP4 (H.264, 1080p, 30 fps)
// and an animated WebP preview for the README.
const fs = require('fs');
const { execFileSync } = require('child_process');

const FAST_SECONDS = 2.5;
const { frames, marks } = JSON.parse(fs.readFileSync('/tmp/liteaura-rec/timeline.json', 'utf8'));

const spans = [];
let open = null;
for (const [type, t] of marks) {
  if (type === 'fast') open = t;
  if (type === 'normal' && open != null) { spans.push([open, t]); open = null; }
}
const factorAt = (t) => {
  for (const [a, b] of spans) if (t >= a && t < b) return Math.max(1, (b - a) / FAST_SECONDS);
  return 1;
};

const lines = [];
let total = 0;
for (let i = 0; i < frames.length - 1; i++) {
  const [file, t] = frames[i];
  const raw = Math.max(0, frames[i + 1][1] - t);
  const d = raw / factorAt(t);
  if (d <= 0) continue;
  lines.push(`file '${file}'`, `duration ${d.toFixed(5)}`);
  total += d;
}
const last = frames[frames.length - 1][0];
lines.push(`file '${last}'`, 'duration 0.5', `file '${last}'`);
fs.writeFileSync('/tmp/liteaura-rec/list.txt', lines.join('\n') + '\n');
console.log('spans', spans.map(([a, b]) => (b - a).toFixed(1)), 'output seconds', total.toFixed(1));

const out = process.argv[2] || '/tmp/liteaura-rec/liteaura-tour.mp4';
execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', '/tmp/liteaura-rec/list.txt',
  '-vf', 'fps=30,format=yuv420p', '-c:v', 'libx264', '-preset', 'slow', '-crf', '21', '-movflags', '+faststart', out], { stdio: 'inherit' });
execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-i', out,
  '-vf', 'fps=8,scale=720:-1:flags=lanczos', '-c:v', 'libwebp', '-quality', '50', '-compression_level', '6', '-loop', '0',
  out.replace(/\.mp4$/, '.webp')], { stdio: 'inherit' });
for (const f of [out, out.replace(/\.mp4$/, '.webp')]) console.log(f, (fs.statSync(f).size / 1e6).toFixed(1), 'MB');
