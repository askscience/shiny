/**
 * audioShared.js — helpers shared by the sound chip (`hudAudio.js`) and the
 * Sound menu (`audioMenu.js`), so a node's icon and label are decided in
 * exactly one place and the chip can never disagree with the menu.
 */

/** The node a panel speaks for: the default one, else the first. */
export function currentNode(nodes, defaultName) {
  const list = nodes || [];
  return list.find((node) => node.is_default)
    || list.find((node) => node.name === defaultName)
    || list[0]
    || null;
}

/** The default sink, as the chip shows it. */
export function currentSink(status) {
  return currentNode(status?.sinks, status?.default_sink);
}

export function isHeadphones(node) {
  const form = (node?.form_factor || '').toLowerCase();
  const icon = (node?.icon || '').toLowerCase();
  return form.includes('headphone') || form.includes('headset') || icon.includes('headphone');
}

/** Theme icon for a node: mute state wins, then form factor, then level. */
export function nodeIcon(node, target) {
  if (!node) return target === 'source' ? 'ui/mic' : 'hud/volume-0';
  if (node.muted) return target === 'source' ? 'ui/mic-off' : 'hud/volume-muted';
  if (isHeadphones(node)) return 'hud/headphones';
  if (target === 'source') return 'ui/mic';
  return `hud/volume-${node.level}`;
}

/** Short label (`Speaker`, `Digital Mic`, …) with the long one as fallback. */
export function nodeLabel(node) {
  return node?.nick || node?.description || '';
}

/** `Apple Audio Device · [Out] Speaker · in use` for a row/detail line. */
export function nodeSubtitle(node) {
  return [
    node?.card,
    node?.active_port,
    (node?.state || '').toUpperCase() === 'RUNNING' ? 'in use' : null,
  ].filter(Boolean).join(' · ');
}
