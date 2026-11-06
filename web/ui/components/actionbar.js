/**
 * actionBar — a window's bottom row of actions, for the thumb.
 *
 * On a phone the top of a window is far from the hand; the actions a user
 * reaches for most (compose, new, upload) sit in one row along the bottom of
 * the window, above the phone's own bar. The row is icon buttons from the
 * shared factories (`toolButton`, `iconButton`); it is hidden anywhere but a
 * phone (ui/ui.css), so a window adds it once and the desktop never sees it.
 *
 *   tile.append(actionBar([
 *     toolButton('ui/plus', composeLabel, compose),
 *   ], { label: actionsLabel }));
 */

export function actionBar(children = [], { className = '', label } = {}) {
  const node = document.createElement('div');
  node.className = `ui-actionbar${className ? ` ${className}` : ''}`;
  node.setAttribute('role', 'toolbar');
  if (label) node.setAttribute('aria-label', label);
  node.append(...[].concat(children).filter(Boolean));
  return node;
}
