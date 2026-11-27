// controls.js — moving the character without a keyboard.
//
//   * A tap or a click on the ground: the character walks there by itself
//     (`hw_tap`, in CSS pixels of the canvas; the game finds the ground).
//   * On a touch screen, a joystick in the bottom left corner: pushed up or
//     down the character walks forward or back, sideways it turns, to the
//     edge it runs (`hw_steer`, each axis -1..1).
//
// The keys (arrows or WASD, Shift to run) are the game's own.

const PAD = 116;
const KNOB = 48;
/** A press that moves less than this, and is short, is a tap. */
const TAP_MOVE = 10;
const TAP_MS = 450;

export function startControls({ game, canvas, lang = 'en' }) {
  // A tap on the ground.
  let down = null;
  canvas.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    down = { x: event.clientX, y: event.clientY, at: performance.now(), id: event.pointerId };
  });
  canvas.addEventListener('pointerup', (event) => {
    if (!down || down.id !== event.pointerId) return;
    const moved = Math.hypot(event.clientX - down.x, event.clientY - down.y);
    const quick = performance.now() - down.at < TAP_MS;
    down = null;
    if (moved > TAP_MOVE || !quick) return;
    const box = canvas.getBoundingClientRect();
    game.hw_tap(event.clientX - box.left, event.clientY - box.top);
  });

  // Any touch screen gets the joystick (some app views report a fine
  // pointer even on a phone).
  const touch = navigator.maxTouchPoints > 0 || 'ontouchstart' in window || window.matchMedia?.('(pointer: coarse)').matches;
  if (!touch) return;
  document.body.classList.add('has-joystick');

  // The joystick.
  const pad = document.createElement('div');
  pad.className = 'joystick';
  pad.setAttribute('role', 'application');
  pad.setAttribute('aria-label', lang.startsWith('it') ? 'Cammina e gira' : 'Walk and turn');
  const knob = document.createElement('div');
  knob.className = 'joystick-knob';
  pad.appendChild(knob);
  document.body.appendChild(pad);

  let held = null;
  const reach = (PAD - KNOB) / 2;
  const move = (event) => {
    const box = pad.getBoundingClientRect();
    let dx = event.clientX - (box.left + box.width / 2);
    let dy = event.clientY - (box.top + box.height / 2);
    const length = Math.hypot(dx, dy);
    if (length > reach) { dx *= reach / length; dy *= reach / length; }
    knob.style.transform = `translate(${dx}px, ${dy}px)`;
    game.hw_steer(-dy / reach, dx / reach);
  };
  const release = () => {
    held = null;
    knob.style.transform = '';
    game.hw_steer(0, 0);
  };
  pad.addEventListener('pointerdown', (event) => {
    event.preventDefault();
    held = event.pointerId;
    pad.setPointerCapture(held);
    move(event);
  });
  pad.addEventListener('pointermove', (event) => { if (event.pointerId === held) move(event); });
  pad.addEventListener('pointerup', (event) => { if (event.pointerId === held) release(); });
  pad.addEventListener('pointercancel', release);
}

export const JOYSTICK = { PAD, KNOB };
