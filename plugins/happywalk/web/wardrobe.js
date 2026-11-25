// wardrobe.js — where the player dresses the character.
//
// The game says what can be chosen (`hw_look_options`, from
// game/src/figure.rs): for each choice a key, and either a list of colours or
// a list of style names. This panel shows them with LiteAura's own controls
// (colours as swatches, styles as chips) and hands every change straight to
// the game, which rebuilds the figure on the spot; the camera is in front of
// it while the panel is open.
//
// A choice added in figure.rs appears here by itself. Its words are looked up
// in WORDS below; one that is not there yet is shown by its name.

import { chip, iconButton, swatches } from '../../ui/index.js';

const WORDS = {
  it: {
    skin: 'Pelle', height: 'Statura', hair: 'Capelli', hair_colour: 'Colore capelli', beard: 'Barba', glasses: 'Occhiali',
    hat: 'Cappello', hat_colour: 'Colore cappello', top: 'Sopra', top_colour: 'Colore', accent: 'Dettagli', scarf: 'Sciarpa',
    bottom: 'Sotto', bottom_colour: 'Colore', shoes: 'Scarpe', backpack: 'Zaino',
    none: 'Niente', small: 'Bassa', medium: 'Media', tall: 'Alta',
    bald: 'Rasati', short: 'Corti', fringe: 'Frangia', long: 'Lunghi', bob: 'Caschetto', ponytail: 'Coda', bun: 'Chignon', curly: 'Ricci', spiky: 'A punte',
    stubble: 'Corta', moustache: 'Baffi', goatee: 'Pizzetto', full: 'Folta',
    round: 'Tondi', sun: 'Da sole',
    beanie: 'Berretto', pompom: 'Con pompon', cap: 'Visiera', sunhat: 'Da sole', bow: 'Fiocco',
    hoodie: 'Felpa', tee: 'Maglietta', jacket: 'Giacca', stripes: 'Righe', dress: 'Vestito',
    trousers: 'Pantaloni', shorts: 'Corti', skirt: 'Gonna',
    random: 'A caso', done: 'Fatto', open: 'Cambia aspetto', colour: 'Colore',
  },
  en: {
    skin: 'Skin', height: 'Height', hair: 'Hair', hair_colour: 'Hair colour', beard: 'Beard', glasses: 'Glasses',
    hat: 'Hat', hat_colour: 'Hat colour', top: 'Top', top_colour: 'Colour', accent: 'Details', scarf: 'Scarf',
    bottom: 'Bottom', bottom_colour: 'Colour', shoes: 'Shoes', backpack: 'Backpack',
    none: 'None', small: 'Small', medium: 'Medium', tall: 'Tall',
    bald: 'Shaved', short: 'Short', fringe: 'Fringe', long: 'Long', bob: 'Bob', ponytail: 'Ponytail', bun: 'Bun', curly: 'Curly', spiky: 'Spiky',
    stubble: 'Stubble', moustache: 'Moustache', goatee: 'Goatee', full: 'Full',
    round: 'Round', sun: 'Sunglasses',
    beanie: 'Beanie', pompom: 'Pompom', cap: 'Cap', sunhat: 'Sun hat', bow: 'Bow',
    hoodie: 'Hoodie', tee: 'T-shirt', jacket: 'Jacket', stripes: 'Stripes', dress: 'Dress',
    trousers: 'Trousers', shorts: 'Shorts', skirt: 'Skirt',
    random: 'Surprise me', done: 'Done', open: 'Change the look', colour: 'Colour',
  },
};

/** The word for `key` in `lang`, in English, or the key itself. */
export function word(lang, key) {
  return WORDS[String(lang).split('-')[0]]?.[key] ?? WORDS.en[key] ?? key;
}

/**
 * Build the wardrobe.
 *
 * `options` is the game's menu; `look` the look to start from; `onChange`
 * gets the whole look after every choice, `onRandom` is asked for a new one,
 * `onDone` closes. Returns the panel element, with `setLook(look)`.
 */
export function createWardrobe({ options, look, lang = 'en', onChange, onRandom, onDone }) {
  const current = { ...look };
  const panel = document.createElement('aside');
  panel.className = 'wardrobe';
  panel.setAttribute('aria-label', word(lang, 'open'));
  const rows = document.createElement('div');
  rows.className = 'wardrobe-rows';
  const controls = new Map();

  for (const option of options) {
    const row = document.createElement('div');
    row.className = 'wardrobe-row';
    const name = document.createElement('span');
    name.className = 'ui-eyebrow';
    name.textContent = word(lang, option.key);
    row.appendChild(name);
    const choose = (value) => {
      current[option.key] = value;
      onChange?.({ ...current });
    };
    if (option.kind === 'style') {
      // Styles are named: a row of chips, one pressed.
      const line = document.createElement('div');
      line.className = 'wardrobe-chips';
      const chips = option.names.map((id, value) => {
        const el = chip({ label: word(lang, id), active: current[option.key] === value, onClick: () => { set(value); choose(value); } });
        line.appendChild(el);
        return el;
      });
      const set = (value) => chips.forEach((el, index) => el.setActive(index === value));
      controls.set(option.key, set);
      row.appendChild(line);
    } else {
      // Colours are shown. An optional one has "none" first (value 0), and
      // its colours count from 1.
      const shift = option.optional ? 1 : 0;
      const list = option.colours.map((color, index) => ({ value: index + shift, color, label: `${word(lang, 'colour')} ${index + 1}` }));
      if (option.optional) list.unshift({ value: 0, label: word(lang, 'none') });
      const control = swatches({ label: word(lang, option.key), options: list, value: current[option.key], onChange: choose });
      controls.set(option.key, (value) => control.setValue(value, { silent: true }));
      row.appendChild(control);
    }
    rows.appendChild(row);
  }

  const foot = document.createElement('div');
  foot.className = 'wardrobe-foot';
  foot.appendChild(iconButton({ icon: 'ui/refresh', label: word(lang, 'random'), onClick: () => onRandom?.() }));
  foot.appendChild(iconButton({ icon: 'ui/check', label: word(lang, 'done'), variant: 'primary', onClick: () => onDone?.({ ...current }) }));
  panel.append(rows, foot);

  panel.setLook = (next) => {
    Object.assign(current, next);
    for (const [key, set] of controls) set(current[key]);
  };
  return panel;
}
