---
status: accepted
date: 2026-11-04
---

# Evidence is an observation; spans are field + UTF-8 byte offsets + quote

Each piece of evidence is one observation of one field at one location (for example the `/body` of `https://github.com/o/p/issues/42`). Its span is a JSON-pointer-style field path, a half-open range of UTF-8 byte offsets, and the quoted text itself. A whole issue body is `/body [0, len)`. No Markdown normalization. This is chosen because evidence can't be rebuilt, so its encoding has to be exact and survive any extractor.

## Re-observation
- When a sync sees content identical to the latest observation at that location and field, the run is linked to the existing evidence (`run_evidence`). No new row is written.
- When the content changed, a new observation is appended, and the old one is kept.
- A→B→A yields three observations.

## Considered options
- **Character offsets:** they depend on the encoding and the language used to count them.
- **Upserting by location** (the prototype did this): it breaks "evidence never changes" and loses history.
