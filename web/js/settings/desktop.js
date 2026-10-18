import { setLayout } from '../desktop.js';
import { select, slider, segmented, revealRegion, disposeMotion } from '../../ui/index.js';
import { t } from '../i18n.js';
import { getDesktopLayout, setDesktopLayout } from '../preferences.js';
import { el, field, sliderField } from './controls.js';

/** Build the desktop section with resources owned by its current mount. */
export function createDesktopSection({ on, cleanups, resources }) {
  function buildDesktop() {
    const layout = getDesktopLayout();

    const mode = segmented({
      label: t('settings.desktop.layout', null, 'Layout'),
      value: layout.mode,
      options: [
        { value: 'master', label: t('settings.desktop.mode.tiling', null, 'Tiling') },
        { value: 'columns', label: t('settings.desktop.mode.columns', null, 'Columns') },
        { value: 'windows', label: t('settings.desktop.mode.windows', null, 'Windows') },
      ],
      onChange: (value) => {
        setDesktopLayout({ ...getDesktopLayout(), mode: value });
        syncTilingControls();
      },
    });
    cleanups.push(() => mode.destroy());

    const orientation = select({
      options: [
        { value: 'left', label: t('common.left', null, 'Left') },
        { value: 'right', label: t('common.right', null, 'Right') },
        { value: 'top', label: t('common.top', null, 'Top') },
        { value: 'bottom', label: t('common.bottom', null, 'Bottom') },
      ],
      onChange: (value) => setDesktopLayout({ ...getDesktopLayout(), orientation: value }),
    });
    orientation.select.value = layout.orientation;

    const ratioControl = slider({
      min: 25, max: 85, step: 5, value: Math.round(layout.master_ratio * 100),
      onInput: (value) => {
        ratio.value.textContent = `${value}%`;
        setDesktopLayout({ ...getDesktopLayout(), master_ratio: value / 100 });
      },
    });
    const ratio = sliderField(t('settings.desktop.masterSize', null, 'Master size'), `${Math.round(layout.master_ratio * 100)}%`, ratioControl);

    const gapControl = slider({
      min: 0, max: 40, step: 2, value: layout.gap,
      onInput: (value) => {
        gap.value.textContent = `${value}px`;
        setLayout({ gap: value });
      },
    });
    const gap = sliderField(t('settings.desktop.gap', null, 'Gap'), `${layout.gap}px`, gapControl);
    gap.wrap.appendChild(el('p', 'settings-hint', t('settings.desktop.gapHint', null, 'Window gaps — and the frame around them: the top bar\u2019s margins, the space to the workspace switcher and the window-to-screen edges.')));

    const tilingControls = el('div', 'settings-tiling-controls');
    cleanups.push(() => disposeMotion(tilingControls));
    tilingControls.append(field(t('settings.desktop.masterSide', null, 'Master side'), orientation), ratio.wrap, gap.wrap);

    const syncTilingControls = () => {
      revealRegion(tilingControls, mode.value !== 'windows');
    };
    syncTilingControls();

    const shortcuts = el('p', 'settings-hint');
    shortcuts.innerHTML = t('settings.desktop.shortcuts', null, 'Shortcuts: <strong>Alt+Enter</strong> fullscreen · <strong>Alt+H/L</strong> focus · <strong>Alt+,/.</strong> workspace · <strong>Alt+1..9</strong> jump · <strong>Alt+N</strong> new · <strong>Alt+Shift+N</strong> remove.');

    return [
      field(t('settings.desktop.layout', null, 'Layout'), mode, {
        details: tilingControls,
        hint: t('settings.desktop.layoutHint', null, 'Tiling snaps windows into workspaces; Windows floats them as draggable, resizable desktop windows.'),
      }),
      shortcuts,
    ];
  }

  /* ── Window surface & top bar (shown in the Appearance panel) ── */


  return buildDesktop();
}
