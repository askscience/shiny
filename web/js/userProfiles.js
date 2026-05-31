const KEY = 'knownUsers';
const MAX_USERS = 8;

export function getKnownUsers() {
  try {
    const users = JSON.parse(localStorage.getItem(KEY) || '[]');
    if (!Array.isArray(users)) return [];
    return users
      .map((u) => ({
        ...u,
        username: u.username || (u.email ? u.email.split('@')[0] : ''),
      }))
      .filter((u) => u.id && u.username);
  } catch {
    return [];
  }
}

export function saveKnownUser(user) {
  if (!user?.id) return;
  const users = getKnownUsers().filter((u) => u.id !== user.id);
  users.unshift({
    id: user.id,
    username: user.username || '',
    name: user.name || user.username || '',
    avatar: user.avatar || null,
  });
  localStorage.setItem(KEY, JSON.stringify(users.slice(0, MAX_USERS)));
}

export function initialsFor(name) {
  const parts = (name || '?').trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return '?';
  return parts.slice(0, 2).map((w) => w[0]?.toUpperCase() || '').join('') || '?';
}

export function renderAvatarEl(el, { name, avatar }) {
  if (!el) return;
  el.innerHTML = '';
  el.classList.toggle('profile-avatar--empty', !avatar);
  if (avatar) {
    const img = document.createElement('img');
    img.src = avatar;
    img.alt = name || '';
    el.appendChild(img);
  } else {
    el.textContent = initialsFor(name);
  }
}

export async function readAvatarFile(file, maxSize = 256) {
  if (!file?.type?.startsWith('image/')) {
    throw new Error('Choose an image file');
  }
  if (file.size > 8 * 1024 * 1024) {
    throw new Error('Image must be under 8 MB');
  }

  const dataUrl = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(new Error('Could not read image'));
    reader.readAsDataURL(file);
  });

  const img = await new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error('Invalid image'));
    image.src = dataUrl;
  });

  const scale = Math.min(1, maxSize / Math.max(img.width, img.height));
  const w = Math.max(1, Math.round(img.width * scale));
  const h = Math.max(1, Math.round(img.height * scale));
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  canvas.getContext('2d').drawImage(img, 0, 0, w, h);
  return canvas.toDataURL('image/jpeg', 0.85);
}
