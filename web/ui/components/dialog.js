import { modal } from './overlay.js';
import { button } from './button.js';
import { input } from './field.js';
import { t } from '../../js/i18n.js';

/** Theme-aware, asynchronous replacements for blocking native browser dialogs. */
function ask(message, { text = false, value = '' } = {}) {
  return new Promise(resolve => {
    let settled = false;
    const finish = result => {
      if (settled) return;
      settled = true;
      dialog.close();
      resolve(result);
    };
    const form = document.createElement('form');
    form.className = 'ui-stack ui-dialog-form';
    const control = text ? input({ value, label: message, autocomplete: 'off' }) : null;
    if (control) form.appendChild(control);
    const actions = document.createElement('div');
    actions.className = 'ui-row ui-dialog-actions';
    // Each action carries its icon, so a phone shows two small round icons
    // (the shared phone button policy) and a desktop reads the words.
    actions.append(
      button({ label: t('common.cancel'), icon: 'ui/close', variant: 'ghost', onClick: () => finish(text ? null : false) }),
      button({ label: t('common.ok'), icon: 'ui/check', type: 'submit' }),
    );
    form.appendChild(actions);
    form.addEventListener('submit', event => { event.preventDefault(); finish(text ? control.value : true); });
    const dialog = modal({ title: message, body: form, removeOnClose: true,
      onClose: () => { if (!settled) { settled = true; resolve(text ? null : false); } },
    });
    dialog.open();
    control?.focus({ preventScroll: true });
    control?.select();
  });
}

export function confirmDialog(message) { return ask(message); }
export function promptDialog(message, value = '') { return ask(message, { text: true, value }); }
