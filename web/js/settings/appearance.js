import { apiFetch } from '../api.js';
import { toast, button, input, select, slider, toggleRow, listThemes, setTheme, getActiveTheme, getThemeManifest, applyAppearance, getAccent, setAccent, getGradient, setGradient, accentPresets, gradientPresets, gradientToCss, getFont, setFont, FONT_THEME, iconsetOptions, getIconsetChoice, setIconset, tintEnabled, setTint, refreshIcons } from '../../ui/index.js';
import { refreshPluginIcons } from '../pluginIcon.js';
import { listFonts } from '../fonts.js';
import { t } from '../i18n.js';
import { getDesktopSurface, setDesktopSurface, getImmersive, setImmersive, getHudChips, setHudChips } from '../preferences.js';
import { SCALE_OPTIONS, getDisplayScale, setDisplayScale } from '../display.js';
import { getBackground, setBackground, renderBackgroundPresets } from '../background.js';
import { pickFiles } from '../files.js';
import { el, field, sliderField, heading } from './controls.js';
import { revealRegion, disposeMotion } from '../../ui/motion.js';

/** Build the appearance section with resources owned by its current mount. */
export function createAppearancePanelSection({ on, cleanups, resources }) {
  const NEUMORPHIC_DARK = 'neumorphic';
  const NEUMORPHIC_LIGHT = 'neumorphic-light';
  const BASE_DARK = 'noir';
  const BASE_LIGHT = 'light';
  function isNeumorphicTheme(name) {
    return name === NEUMORPHIC_DARK || name === NEUMORPHIC_LIGHT;
  }

  function isLightTheme(name) {
    return name === NEUMORPHIC_LIGHT || name === BASE_LIGHT;
  }

  function buildAccentSwatches(container) {
    container.textContent = '';
    const current = getAccent();
    const presets = accentPresets();

    for (const preset of presets) {
      const b = el('button', 'swatch');
      b.type = 'button';
      b.dataset.accent = preset.value;
      b.style.background = preset.value;
      b.title = preset.label;
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-label', preset.label);
      on(b, 'click', () => {
        setAccent(preset.value);
        applyAppearance();
        markActive(container, (n) => n.dataset.accent === preset.value);
      });
      container.appendChild(b);
    }

    const custom = el('label', 'swatch swatch--custom');
    custom.dataset.accent = 'custom';
    custom.title = t('settings.appearance.customAccent', null, 'Custom accent');
    custom.appendChild(el('span', null, '+'));
    const color = input({ type: 'color' });
    color.value = presets.some((p) => p.value === current) ? presets[0]?.value || '#ffffff' : current;
    color.setAttribute('aria-label', t('settings.appearance.customAccentColor', null, 'Custom accent color'));
    on(color, 'input', () => {
      setAccent(color.value);
      applyAppearance();
      markActive(container, (n) => n.dataset.accent === current || n.dataset.accent === 'custom');
    });
    custom.appendChild(color);
    container.appendChild(custom);

    const isPreset = presets.some((p) => p.value === current);
    markActive(container, (n) =>
      n.dataset.accent === current || (n.dataset.accent === 'custom' && !isPreset));
  }

  function buildGradientSwatches(container) {
    container.textContent = '';
    const current = getGradient();
    for (const preset of gradientPresets()) {
      const b = el('button', 'swatch swatch--gradient');
      b.type = 'button';
      b.dataset.gradientId = preset.id;
      b.style.background = gradientToCss(preset);
      b.title = preset.label;
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-label', preset.label);
      on(b, 'click', () => {
        setGradient({ id: preset.id, angle: preset.angle, stops: preset.stops });
        applyAppearance();
        markActive(container, (n) => n.dataset.gradientId === preset.id);
      });
      container.appendChild(b);
    }
    markActive(container, (n) => n.dataset.gradientId === current.id);
  }

  function markActive(container, isActive) {
    container?.querySelectorAll('.swatch').forEach((node) => {
      node.classList.toggle('is-active', isActive(node));
    });
  }

  /* ── Background ─────────────────────────────────────────────── */

  function buildBackgroundControls() {
    const mode = select({
      options: [
        { value: 'none', label: t('settings.appearance.background.none', null, 'None (default mesh)') },
        { value: 'gradient', label: t('settings.appearance.background.gradient', null, 'Gradient') },
        { value: 'image', label: t('settings.appearance.background.image', null, 'Image') },
        { value: 'animated', label: t('settings.appearance.background.animated', null, 'Animated') },
      ],
      onChange: (value) => {
        setBackground({ mode: value });
        syncBackground();
      },
    });
    const gradientNote = el('div', 'settings-hint hidden', t('settings.appearance.background.gradientNote', null, 'Gradient uses your Appearance gradient — change it under Appearance.'));

    const presetGrid = el('div', 'bg-preset-grid');
    presetGrid.setAttribute('role', 'radiogroup');
    const pickBackgroundImage = async () => {
      const [file] = await pickFiles({ accept: 'image/*' });
      if (file) void uploadBackground(file, syncBackground);
    };
    const uploadBtn = button({ label: t('settings.appearance.background.upload', null, 'Upload your own…'), variant: 'quiet', size: 'sm' });
    const removeBtn = button({ label: t('settings.appearance.background.remove', null, 'Remove photo'), variant: 'danger', size: 'sm' });
    removeBtn.classList.add('hidden');
    const imageRow = el('div', 'background-image-row');
    imageRow.append(uploadBtn, removeBtn);
    const imageHint = el('p', 'settings-hint');

    const imageControls = el('div', 'hidden');
    imageControls.append(presetGrid, imageRow, imageHint);

    const animSelect = select({
      options: [
        { value: 'aurora', label: t('settings.appearance.background.aurora', null, 'Aurora drift') },
        { value: 'shimmer', label: t('settings.appearance.background.shimmer', null, 'Shimmer') },
      ],
      onChange: (value) => setBackground({ animation: value }),
    });
    const animControls = el('div', 'hidden');
    cleanups.push(() => { disposeMotion(gradientNote); disposeMotion(imageControls); disposeMotion(animControls); });
    animControls.append(field(t('settings.appearance.background.animation', null, 'Animation'), animSelect));

    const syncBackground = () => {
      const bg = getBackground();
      mode.select.value = bg.mode || 'none';
      revealRegion(gradientNote, bg.mode === 'gradient');
      revealRegion(imageControls, bg.mode === 'image');
      revealRegion(animControls, bg.mode === 'animated');
      animSelect.select.value = bg.animation || 'aurora';
      removeBtn.classList.toggle('hidden', !bg.url);
      imageHint.textContent = bg.mode === 'image'
        ? (bg.url
          ? t('settings.appearance.background.photoHint', null, 'Your photo is dimmed so the interface stays readable.')
          : t('settings.appearance.background.wallpaperHint', null, 'Built-in wallpapers. Pick one, or upload your own image — it is dimmed so the interface stays readable.'))
        : '';

      renderBackgroundPresets(presetGrid, {
        onSelect: (preset) => {
          setBackground({ mode: 'image', preset: preset.id, url: null });
          syncBackground();
        },
        onUpload: () => void pickBackgroundImage(),
      });
    };

    on(uploadBtn, 'click', () => void pickBackgroundImage());
    on(removeBtn, 'click', () => void removeBackgroundImage(syncBackground));

    syncBackground();
    const backgroundDetails = el('div');
    backgroundDetails.append(gradientNote, imageControls, animControls);
    return [
      field(t('settings.appearance.background.type', null, 'Type'), mode, { details: backgroundDetails }),
    ];
  }

  async function uploadBackground(file, sync) {
    if (!file.type?.startsWith('image/')) {
      toast(t('settings.appearance.background.chooseImage', null, 'Choose an image file'), { type: 'error' });
      return;
    }
    if (file.size > 12 * 1024 * 1024) {
      toast(t('settings.appearance.background.tooLarge', null, 'Image must be under 12 MB'), { type: 'error' });
      return;
    }
    try {
      const form = new FormData();
      form.append('file', file);
      await apiFetch('/api/background', { method: 'POST', body: form });
      setBackground({ mode: 'image', url: `/api/background?v=${Date.now()}`, preset: null });
      sync();
      toast(t('settings.appearance.background.updated', null, 'Background image updated'), { type: 'info' });
    } catch (e) {
      toast(e.message || t('settings.appearance.background.uploadFailed', null, 'Could not upload image'), { type: 'error' });
    }
  }

  async function removeBackgroundImage(sync) {
    try {
      await apiFetch('/api/background', { method: 'DELETE' });
    } catch (_) { /* 404 or transient — proceed */ }
    setBackground({ mode: 'image', url: null, preset: null });
    sync();
    toast(t('settings.appearance.background.removed', null, 'Background image removed'), { type: 'info' });
  }

  /* ── Account ────────────────────────────────────────────────── */

  function buildWindowSurfaceControls() {
    const surface = getDesktopSurface();
    const titleControl = slider({
      min: 28, max: 48, step: 1, value: surface.title_height,
      onInput: (value) => {
        title.value.textContent = `${value}px`;
        setDesktopSurface({ title_height: value });
      },
    });
    const title = sliderField(t('settings.appearance.titleBarHeight', null, 'Title bar height'), `${surface.title_height}px`, titleControl);

    const radiusControl = slider({
      min: 0, max: 32, step: 1, value: surface.window_radius,
      onInput: (value) => {
        radius.value.textContent = `${value}px`;
        setDesktopSurface({ window_radius: value });
      },
    });
    const radius = sliderField(t('settings.appearance.windowCorners', null, 'Window corners'), `${surface.window_radius}px`, radiusControl);

    const opacityControl = slider({
      min: 60, max: 100, step: 1, value: surface.window_opacity,
      onInput: (value) => {
        opacity.value.textContent = `${value}%`;
        setDesktopSurface({ window_opacity: value });
      },
    });
    const opacity = sliderField(t('settings.appearance.windowOpacity', null, 'Window opacity'), `${surface.window_opacity}%`, opacityControl);

    const blurControl = slider({
      min: 0, max: 24, step: 1, value: surface.glass_blur,
      onInput: (value) => {
        blur.value.textContent = `${value}px`;
        setDesktopSurface({ glass_blur: value });
      },
    });
    const blur = sliderField(t('settings.appearance.glassBlur', null, 'Glass blur'), `${surface.glass_blur}px`, blurControl);

    const background = toggleRow({
      label: t('settings.appearance.windowBackground', null, 'Blurred window background'),
      hint: t('settings.appearance.windowBackgroundHint', null, 'The soft blurred mirror behind a window — its photo, PDF page, artwork or the accent glow. Turn off for flat, fully opaque-looking windows.'),
      checked: surface.window_background,
      onChange: (checked) => setDesktopSurface({ window_background: checked }),
    });

    const shadow = toggleRow({
      label: t('settings.appearance.windowShadows', null, 'Window shadows'),
      hint: t('settings.appearance.windowShadowsHint', null, 'Give windows a soft drop shadow instead of a flat hairline.'),
      checked: surface.window_shadow,
      onChange: (checked) => setDesktopSurface({ window_shadow: checked }),
    });

    return [
      heading(t('settings.appearance.windowSurface', null, 'Window surface')),
      title.wrap,
      radius.wrap,
      opacity.wrap,
      blur.wrap,
      background,
      shadow,
    ];
  }

  function buildTopBarControls() {
    const immersive = getImmersive();
    const autohideBar = toggleRow({
      label: t('settings.appearance.autohideBar', null, 'Autohide the top bar'),
      hint: t('settings.appearance.autohideBarHint', null, 'Slide the top bar away until the pointer reaches the edge it docks to.'),
      checked: immersive.autohide_bar,
      onChange: (checked) => setImmersive({ autohide_bar: checked }),
    });
    const barPosition = select({
      options: [
        { value: 'top', label: t('common.top', null, 'Top') },
        { value: 'left', label: t('common.left', null, 'Left') },
        { value: 'right', label: t('common.right', null, 'Right') },
        { value: 'bottom', label: t('common.bottom', null, 'Bottom') },
        { value: 'center', label: t('common.center', null, 'Center') },
      ],
      onChange: (value) => setImmersive({ bar_position: value }),
    });
    barPosition.select.value = immersive.bar_position;
    const autohideOrb = toggleRow({
      label: t('settings.appearance.autohideOrb', null, 'Autohide the voice bar'),
      hint: t('settings.appearance.autohideOrbHint', null, 'Slide the voice bar away until the pointer reaches the edge it docks to.'),
      checked: immersive.autohide_orb,
      onChange: (checked) => setImmersive({ autohide_orb: checked }),
    });
    const orbPosition = select({
      options: [
        { value: 'bottom-center', label: t('settings.appearance.orbPosition.bottomCenter', null, 'Bottom centre') },
        { value: 'bottom-left', label: t('settings.appearance.orbPosition.bottomLeft', null, 'Bottom left') },
        { value: 'bottom-right', label: t('settings.appearance.orbPosition.bottomRight', null, 'Bottom right') },
        { value: 'top-left', label: t('settings.appearance.orbPosition.topLeft', null, 'Top left (pill in the top bar)') },
      ],
      onChange: (value) => setImmersive({ orb_position: value }),
    });
    orbPosition.select.value = immersive.orb_position;

    return [
      heading(t('settings.appearance.topBarHeading', null, 'Top bar & voice bar')),
      autohideBar,
      field(t('settings.appearance.topBarPosition', null, 'Top bar position'), barPosition),
      autohideOrb,
      field(t('settings.appearance.orbPosition', null, 'Voice bar position'), orbPosition, {
        hint: t('settings.appearance.orbPositionHint', null,
          'At the top left the voice bar becomes a pill in the top bar row, like the workspace chooser. '
          + 'With the top bar at the bottom, a bottom-docked voice bar moves to that top-left pill instead.'),
      }),
    ];
  }

  /* Content of the top-bar host status chips: the device label and the numeric
   * percentage (volume / signal / battery) can each be hidden. Both off leaves
   * an icon-only button — see hudChipsShared.js + preferences.js. */
  function buildHudChipControls() {
    const chips = getHudChips();
    const names = toggleRow({
      label: t('settings.appearance.deviceNames', null, 'Device names'),
      hint: t('settings.appearance.deviceNamesHint', null, 'Show the device or network name in the sound, network, Bluetooth and battery buttons.'),
      checked: chips.name,
      onChange: (checked) => setHudChips({ name: checked }),
    });
    const percent = toggleRow({
      label: t('settings.appearance.percentages', null, 'Percentages'),
      hint: t('settings.appearance.percentagesHint', null, 'Show volume, signal and battery numbers. Turn both off for icon-only buttons.'),
      checked: chips.percent,
      onChange: (checked) => setHudChips({ percent: checked }),
    });

    return [
      heading(t('settings.appearance.hostStatusChips', null, 'Host status chips')),
      el('p', 'settings-hint', t('settings.appearance.hostStatusChipsHint', null, 'Choose what the sound, network, Bluetooth and battery buttons in the top bar show.')),
      names,
      percent,
    ];
  }

  /* ── System ─────────────────────────────────────────────────── */

  function buildAppearancePanel() {
    const theme = select({
      onChange: async (value) => {
        await setTheme(value);
        syncNeumorphic();
        buildAccentSwatches(accentSwatches);
        buildGradientSwatches(gradientSwatches);
      },
    });
    const neumorphic = toggleRow({
      label: t('settings.appearance.neumorphic', null, 'Neumorphic soft UI'),
      hint: t('settings.appearance.neumorphicHint', null, 'Soft extruded-plastic surfaces with subtle dual shadows, in dark and light.'),
      checked: isNeumorphicTheme(getActiveTheme()),
      onChange: () => void toggleNeumorphic(),
    });

    const iconset = select({
      options: iconsetOptions().map(({ value, label }) => ({ value, label })),
      onChange: (value) => {
        setIconset(value);
        refreshIcons();
        refreshPluginIcons();
      },
    });
    iconset.select.value = getIconsetChoice();
    const iconsetField = field(t('settings.appearance.iconset', null, 'Icon set'), iconset, {
      hint: iconsetOptions().find((o) => o.value === getIconsetChoice())?.hint
        || t('settings.appearance.iconsetHint', null, 'Infinity is coloured; Slot-Beauty is the monochrome line set. Themes may still override individual icons.'),
    });

    const tintIcons = toggleRow({
      label: t('settings.appearance.tintedIcons', null, 'Accent-tinted icons'),
      hint: t('settings.appearance.tintedIconsHint', null, 'Recolour the set’s artwork (folders, app and file icons) to your accent. Off keeps each set’s native colours.'),
      checked: tintEnabled(),
      onChange: (checked) => {
        setTint(checked);
        refreshIcons();
        refreshPluginIcons();
      },
    });

    const accentSwatches = el('div', 'swatch-row');
    accentSwatches.setAttribute('role', 'radiogroup');
    const gradientSwatches = el('div', 'swatch-row swatch-row--gradient');
    gradientSwatches.setAttribute('role', 'radiogroup');

    const stopA = input({ type: 'color' });
    stopA.value = '#ffffff';
    const stopB = input({ type: 'color' });
    stopB.value = '#8a8a8a';
    const angle = slider({ min: 0, max: 360, step: 5, value: 135, onInput: () => applyCustomGradient() });
    const customControls = el('div', 'gradient-custom-controls');
    customControls.append(stopA, stopB, angle);
    const custom = el('details', 'gradient-custom');
    custom.appendChild(el('summary', null, t('settings.appearance.customGradient', null, 'Custom gradient')));
    custom.appendChild(customControls);

    function applyCustomGradient() {
      setGradient({ id: 'custom', stops: [stopA.value, stopB.value], angle: Number(angle.value) });
      applyAppearance();
      markActive(gradientSwatches, (n) => n.dataset.gradientId === 'custom');
    }
    on(stopA, 'input', applyCustomGradient);
    on(stopB, 'input', applyCustomGradient);
    on(angle, 'input', applyCustomGradient);

    const font = select({
      options: [{ value: getFont(), label: getFont() }],
      onChange: (value) => setFont(value),
    });
    // Populated from the host's installed fonts (GET /api/fonts). The current
    // choice is kept even when it is not installed here, so a preference carried
    // from another machine never silently resets.
    void listFonts().then((families) => {
      const values = families.slice();
      const current = getFont();
      if (current !== FONT_THEME && !values.includes(current)) values.unshift(current);
      font.setOptions([
        ...values.map((f) => ({ value: f, label: f })),
        { value: FONT_THEME, label: t('settings.appearance.themeDefault', null, 'Theme default') },
      ]);
      font.select.value = getFont();
    });
    font.select.value = getFont();
    const fontField = field(t('settings.appearance.globalFont', null, 'Global font'), font, {
      hint: t('settings.appearance.globalFontHint', null, 'Every installed system font is listed. "Theme default" restores the active theme’s own fonts.'),
    });

    const backgroundChildren = buildBackgroundControls();
    const windowSurfaceChildren = buildWindowSurfaceControls();
    const topBarChildren = buildTopBarControls();
    const hudChipChildren = buildHudChipControls();

    // Interface scale: a host setting, applied by the kiosk shell as page zoom.
    // Changing it re-lays-out the page, so the Terminal re-fits automatically.
    const scale = select({
      options: SCALE_OPTIONS.map((o) => ({
        value: o.value,
        label: o.value === 'auto' ? t('settings.appearance.scale.auto', null, o.label) : o.label,
      })),
      onChange: (value) => void applyScale(value),
    });
    const scaleField = field(t('settings.appearance.interfaceScale', null, 'Interface scale'), scale);
    const scaleHint = el('p', 'settings-hint');
    scaleField.appendChild(scaleHint);

    async function refreshScale() {
      try {
        const data = await getDisplayScale();
        const stored = data?.scale ?? 'auto';
        scale.select.value = String(stored);
        const resolved = data?.resolved;
        const pct = typeof resolved === 'number' && Number.isFinite(resolved)
          ? Math.round(resolved * 100) : null;
        const dpi = data?.dpi;
        const panel = typeof dpi === 'number' && dpi > 0
          ? t('settings.appearance.scale.panel', { dpi: Math.round(dpi) }, ` on this ${Math.round(dpi)}-DPI panel`)
          : '';
        scaleHint.textContent = pct
          ? t('settings.appearance.scale.applied', { percent: pct, panel }, `The shell applies ${pct}%${panel}. Scales the whole desktop. Auto follows this panel's pixel density — the right default on a high-DPI screen, and it also cuts rendering cost.`)
          : t('settings.appearance.scale.hint', null, `Scales the whole desktop. Auto follows this panel's pixel density — the right default on a high-DPI screen, and it also cuts rendering cost.`);
      } catch (_) {
        scaleHint.textContent = t('settings.appearance.scale.readFailed', null, 'Could not read the current scale.');
      }
    }

    async function applyScale(value) {
      try {
        await setDisplayScale(value);
        await refreshScale();
        toast(t('settings.appearance.scale.updated', null, 'Interface scale updated'));
      } catch (err) {
        toast(err?.message || t('settings.appearance.scale.updateFailed', null, 'Could not change the interface scale'), { type: 'error' });
        void refreshScale();
      }
    }

    function syncNeumorphic() {
      neumorphic.toggle.setChecked(isNeumorphicTheme(getActiveTheme()), { silent: true });
    }

    async function toggleNeumorphic() {
      const current = getActiveTheme();
      const next = isNeumorphicTheme(current)
        ? (isLightTheme(current) ? BASE_LIGHT : BASE_DARK)
        : (isLightTheme(current) ? NEUMORPHIC_LIGHT : NEUMORPHIC_DARK);
      const ok = await setTheme(next);
      if (!ok) return;
      neumorphic.toggle.setChecked(isNeumorphicTheme(next), { silent: true });
      buildAccentSwatches(accentSwatches);
      buildGradientSwatches(gradientSwatches);
    }

    // Deferred so the swatches exist in the DOM before they are built.
    queueMicrotask(() => {
      buildAccentSwatches(accentSwatches);
      buildGradientSwatches(gradientSwatches);
      custom.open = getGradient().id === 'custom';
      const g = getGradient();
      if (g.id === 'custom' && g.stops?.length >= 2) {
        stopA.value = g.stops[0];
        stopB.value = g.stops[1];
        if (Number.isFinite(g.angle)) angle.value = String(g.angle);
      }
      void listThemes().then((themes) => {
        theme.setOptions(themes.map((name) => ({
          value: name,
          label: name === getActiveTheme() ? (getThemeManifest()?.label || name) : name.charAt(0).toUpperCase() + name.slice(1),
        })));
        theme.select.value = getActiveTheme();
      });
      void refreshScale();
    });

    return [
      field(t('settings.appearance.theme', null, 'Theme'), theme),
      iconsetField,
      tintIcons,
      neumorphic,
      field(t('settings.appearance.accent', null, 'Accent'), accentSwatches),
      field(t('settings.appearance.gradient', null, 'Gradient'), gradientSwatches),
      custom,
      fontField,
      heading(t('common.display', null, 'Display')),
      scaleField,
      ...windowSurfaceChildren,
      ...topBarChildren,
      ...hudChipChildren,
      heading(t('settings.appearance.background.heading', null, 'Background')),
      el('p', 'settings-hint', t('settings.appearance.background.hint', null, 'The full-screen backdrop behind the desktop — the default mesh, your gradient, a photo, or a subtle animation.')),
      ...backgroundChildren,
    ];
  }


  return buildAppearancePanel();
}
