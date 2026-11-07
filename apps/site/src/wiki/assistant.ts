import { eligibility, localModel } from './assistant-model';
import { takeTrail, type Topic } from './assistant-context';

/** Eligibility checks never download a model. Unsupported readers see no controls or notices. */
export async function setupAssistant(): Promise<void> {
  const state = await eligibility();
  if (!['available', 'downloadable', 'downloading'].includes(state)) return;
  const model = localModel();
  if (!model) return;
  const entries = new Map<string, { topic: Topic; heading: HTMLElement; button: HTMLButtonElement }>();
  let ui: Promise<typeof import('./assistant-card')> | undefined;
  let request = 0;
  const load = () => ui ??= import('./assistant-card').catch((error) => { ui = undefined; throw error; });
  for (const heading of document.querySelectorAll<HTMLElement>('.wk-doc .wk-sec > h2, .wk-doc .wk-sec h3[id]')) {
    const id = heading.tagName === 'H2' ? heading.parentElement?.id : heading.id;
    if (!id) continue;
    const topic = { url: location.pathname + '#' + id, title: heading.textContent?.trim() ?? 'This section', level: Number(heading.tagName[1]) };
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'wk-help-open';
    const mark = document.createElement('span');
    mark.textContent = '?';
    mark.setAttribute('aria-hidden', 'true');
    button.append(mark);
    button.title = 'Understand this section';
    button.setAttribute('aria-label', 'Understand ' + topic.title);
    button.setAttribute('aria-expanded', 'false');
    const entry = { topic, heading, button };
    entries.set(topic.url, entry);
    button.addEventListener('click', async () => {
      const current = ++request;
      try {
        const module = await load();
        if (request === current) module.openCard(entry, model, state, entries);
      } catch { button.title = 'Explanation could not load. Try again.'; }
    });
    heading.append(button);
  }
  const trail = takeTrail(location.pathname + location.hash);
  if (trail && entries.has(trail.to)) {
    const current = ++request;
    try {
      const module = await load();
      if (request === current) module.openCard(entries.get(trail.to)!, model, state, entries, trail);
    } catch { /* the destination remains an ordinary readable wiki section */ }
  }
}
