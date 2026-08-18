/**
 * Shiny UI — unified component library.
 *
 * Theme-agnostic engine. Structure lives in /ui/ui.css; every visual
 * decision comes from the active theme in /themes/<name>/. Plugin
 * content (artifacts, insights) is rendered through these components,
 * so plugins always match the user's theme and accent.
 */

export { initThemeLoader, listThemes, setTheme, getActiveTheme, getThemeManifest, themeUrl } from './theme-loader.js';
export {
  initAppearance, refreshAppearance, applyAppearance,
  getAccent, setAccent, getGradient, setGradient,
  accentPresets, gradientPresets, gradientToCss,
  hexToRgb, rgba, contrastFor, cssVar,
} from './appearance.js';
export { icon, setIcon, clearIconCache, hydrateIcons } from './icon.js';
export { reveal, revealNow } from './reveal.js';

export { button, iconButton } from './components/button.js';
export { field, input, textarea, select, toggle, toggleRow, slider, checkbox, searchBar } from './components/field.js';
export { card, panel, section, divider, stack, row } from './components/card.js';
export { modal, sheet, tooltip } from './components/overlay.js';
export { toast, wireToastEvents, spinner, skeleton, progress, emptyState, badge } from './components/feedback.js';
export { list, listItem, stat, chip, avatar, keyValue } from './components/data.js';
export {
  dockButton, insightCard, artifactPanel,
  iconForArtifact, labelForArtifact,
} from './components/composites.js';
