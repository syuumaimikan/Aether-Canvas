// Print every file a model3.json refers to, one per line.
import fs from 'node:fs';
const m = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const r = m.FileReferences ?? {};
const files = [r.Moc, ...(r.Textures ?? []), r.Physics, r.Pose, r.DisplayInfo, r.UserData];
for (const e of r.Expressions ?? []) files.push(e.File);
for (const group of Object.values(r.Motions ?? {})) for (const e of group) files.push(e.File, e.Sound);
for (const f of files) if (f) console.log(f);
