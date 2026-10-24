// Build-time only. The shipped admin never contacts a third party.
// Regenerate deliberately after reviewing the pinned package and its notices.
import { writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const base = 'https://unpkg.com/emojibase-data@17.0.0/';
const files = ['en/compact.json', 'LICENSE'];
const expected = {
  'en/compact.json':'1c7cf539328563743c2b2adff7117ccff0ae6852d591366f4363f50503d5e398',
  LICENSE:'5c4e1582dcae9429a5d00ec0a64185c3a711f9757b2308fdf275d90b120f6149'
};
for (const file of files) {
  const response = await fetch(base + file);
  if (!response.ok) throw new Error(`${file}: ${response.status}`);
  const text = await response.text();
  const hash = createHash('sha256').update(text).digest('hex');
  if (hash !== expected[file]) throw new Error(`Upstream integrity changed: ${file}`);
  if (file.endsWith('.json')) {
    const data = JSON.parse(text);
    if (data.length < 1500 || !data.some((entry) => entry.skins?.length)) throw new Error('Incomplete emoji dataset');
  }
  const target = file.endsWith('.json') ? 'admin-emoji-data.json' : 'admin-emoji-LICENSE.txt';
  await writeFile(new URL(target, import.meta.url), text);
  console.log(`${target} SHA-256 ${createHash('sha256').update(text).digest('hex')}`);
}
