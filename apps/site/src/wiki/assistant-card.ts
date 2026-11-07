import { retrieve, inTopic, saveTrail, type Corpus, type Passage, type Reply, type Topic, type Trail, type Turn } from './assistant-context';
import { createSession, explain, type Availability, type LocalModel } from './assistant-model';

interface Entry { topic: Topic; heading: HTMLElement; button: HTMLButtonElement }
let active: { entry: Entry; close: (focus?: boolean) => void } | undefined;
let corpusPromise: Promise<Corpus> | undefined;
const corpus = () => corpusPromise ??= import('virtual:wiki-passages').then((m) => m.default)
  .catch((error) => { corpusPromise = undefined; throw error; });

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  el.className = className;
  if (text) el.textContent = text;
  return el;
}
function button(text: string, className = 'wk-help-option'): HTMLButtonElement {
  const el = element('button', className, text);
  el.type = 'button';
  return el;
}
function suggestions(entry: Entry) {
  let node = entry.heading.nextElementSibling;
  let term = '';
  for (let i = 0; node && i < 4; i++, node = node.nextElementSibling) {
    if (/^H[23]$/.test(node.tagName)) break;
    term = [...node.querySelectorAll('code')].map((c) => c.textContent?.trim() ?? '')
      .find((t) => t.length >= 3 && t.length <= 30 && /^[\w:.-]+$/.test(t)) ?? '';
    if (term) break;
  }
  const name = entry.topic.title;
  return [
    { label: name.length < 35 ? 'Explain “' + name + '”' : 'Explain in plain words', question: 'Explain "' + name + '" in plain language. What is the main idea?' },
    term ? { label: 'What is ' + term + '?', question: 'What does "' + term + '" mean in this section, and why is it needed?' } :
      { label: 'Walk me through it', question: 'Walk me through a concrete example of "' + name + '", using only what this section establishes.' },
    { label: 'Where does this fit?', question: 'How does "' + name + '" connect to the rest of hoi? Which section would help me understand the next part?' },
  ];
}

export function openCard(entry: Entry, model: LocalModel, state: Availability, entries: Map<string, Entry>, incoming?: Trail): void {
  if (active?.entry === entry && !incoming) { active.close(true); return; }
  active?.close();
  const choices = suggestions(entry);
  const card = element('section', 'wk-help');
  card.id = 'wk-section-explanation';
  card.setAttribute('aria-labelledby', 'wk-help-title');
  const head = element('div', 'wk-help-head');
  const title = element('strong', 'wk-help-title', 'A closer look');
  title.id = 'wk-help-title';
  const close = button('×', 'wk-help-close');
  close.setAttribute('aria-label', 'Close explanation');
  head.append(title, close);
  const intro = element('p', 'wk-help-intro', 'Make sense of this section, then follow the connections.');
  const note = element('p', 'wk-help-note', state === 'available'
    ? 'On-device AI · Check the linked sources.'
    : 'On-device AI · Your first question starts Chrome’s model download. It may take a while.');
  const details = element('details', 'wk-help-about');
  details.append(element('summary', '', 'About this helper'));
  details.append(element('p', '', 'Questions and answers stay on this device. Chrome downloads and manages the AI model. Following an explanation link briefly keeps the context in this tab so you can continue; it is used once, expires after 15 minutes and is never put in a URL. AI can make mistakes.'));
  const options = element('div', 'wk-help-options');
  options.setAttribute('aria-label', 'Ways to understand ' + entry.topic.title);
  const result = element('div', 'wk-help-result');
  result.setAttribute('aria-live', 'polite');
  const form = element('form', 'wk-help-form');
  const label = element('label', '', 'Ask about this section');
  label.htmlFor = 'wk-help-question';
  const input = element('textarea', 'wk-help-input');
  input.id = label.htmlFor;
  input.rows = 2;
  input.maxLength = 1200;
  input.placeholder = 'What is still unclear?';
  const actions = element('div', 'wk-help-actions');
  const submit = button('Ask', 'wk-help-ask');
  submit.type = 'submit';
  submit.disabled = true;
  const stop = button('Stop', 'wk-help-stop');
  stop.hidden = true;
  const status = element('p', 'wk-help-status');
  status.setAttribute('role', 'status');
  status.setAttribute('aria-live', 'polite');
  status.setAttribute('aria-atomic', 'true');
  actions.append(submit, stop, status);
  form.append(label, input, actions);
  card.append(head, intro, note, options, result, form, details);
  entry.heading.insertAdjacentElement('afterend', card);
  entry.button.setAttribute('aria-expanded', 'true');
  entry.button.setAttribute('aria-controls', card.id);

  let disposed = false;
  let busy = false;
  let controller: AbortController | undefined;
  let history: Turn[] = [];
  let trail: Trail | undefined;
  let original = '';
  let run = 0;
  const setStatus = (text: string) => { if (!disposed) status.textContent = text; };
  const setBusy = (value: boolean) => {
    busy = value;
    submit.disabled = value || !input.value.trim();
    options.querySelectorAll('button').forEach((b) => { b.disabled = value; });
    stop.hidden = !value;
    result.setAttribute('aria-busy', String(value));
  };
  const shut = (focus = false) => {
    disposed = true;
    ++run;
    controller?.abort();
    card.remove();
    entry.button.setAttribute('aria-expanded', 'false');
    entry.button.removeAttribute('aria-controls');
    if (active?.entry === entry) active = undefined;
    window.removeEventListener('pagehide', onLeave);
    window.removeEventListener('popstate', onLocation);
    window.removeEventListener('hashchange', onLocation);
    if (focus) entry.button.focus({ preventScroll: true });
  };
  const onLeave = () => {
    controller?.abort();
    setBusy(false);
    setStatus('Ask a question to continue.');
  };
  const onLocation = () => {
    if (location.pathname + location.hash !== entry.topic.url) shut();
  };
  window.addEventListener('pagehide', onLeave);
  window.addEventListener('popstate', onLocation);
  window.addEventListener('hashchange', onLocation);
  active = { entry, close: shut };
  close.addEventListener('click', () => shut(true));
  card.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); shut(true); }
  });
  stop.addEventListener('click', () => { controller?.abort(); });
  input.addEventListener('input', () => { submit.disabled = busy || !input.value.trim(); });
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    if (!busy && input.value.trim()) void ask(input.value.trim());
  });

  function followLink(destination: Passage, reason: string, question: string, answer: string, version: string, labelText: string): HTMLAnchorElement {
    const link = element('a', 'wk-help-next');
    link.href = destination.url;
    link.append(element('span', 'wk-help-next-label', labelText), element('strong', '', destination.title),
      element('span', 'wk-help-next-page', destination.page), element('span', '', reason));
    link.addEventListener('click', (event) => {
      // Modified clicks keep native link behavior and do not leave context in the old tab.
      if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
      const next: Trail = {
        version, original: (original || question).slice(0, 1200),
        question: question.slice(0, 1200), answer: answer.slice(0, 4000),
        from: entry.topic.url, fromTitle: entry.topic.title.slice(0, 300), to: destination.url, at: Date.now(),
      };
      const target = entries.get(destination.url);
      if (target) {
        event.preventDefault();
        historyPush(destination.url);
        openCard(target, model, 'available', entries, next);
      } else {
        saveTrail(next);
      }
    });
    return link;
  }
  function showReply(reply: Reply, passages: Passage[], data: Corpus, question: string) {
    result.replaceChildren();
    const asked = element('p', 'wk-help-asked', question);
    const answer = element('div', 'wk-help-answer');
    for (const paragraph of reply.answer.split(/\n+/).filter(Boolean)) answer.append(element('p', '', paragraph));
    result.append(asked, answer);
    if (reply.sources.length) {
      const sources = element('p', 'wk-help-sources');
      sources.append('Based on ');
      const seen = new Set<string>();
      for (const n of reply.sources) {
        const p = passages[n - 1];
        if (seen.has(p.url)) continue;
        if (seen.size) sources.append(' · ');
        seen.add(p.url);
        const link = element('a', '', p.title);
        link.href = p.url;
        sources.append(link);
      }
      result.append(sources);
    }
    const next = element('div', 'wk-help-connections');
    for (const item of reply.next) {
      next.append(followLink(passages[item.source - 1], item.reason, item.question, reply.answer, data.version, 'Continue here →'));
    }
    if (reply.status === 'not-covered') {
      // Deterministic suggestions are explicitly navigation, never presented as generated evidence.
      for (const p of passages.filter((p) => !inTopic(p, entry.topic)).slice(0, 2)) {
        next.append(followLink(p, 'This related section may help you investigate.', question, reply.answer, data.version, 'Explore this section →'));
      }
    }
    if (next.childElementCount) result.append(next);
    else if (reply.status === 'answered') result.append(element('p', 'wk-help-finish', 'You can keep reading here, or ask about the part that is still unclear.'));
  }
  async function ask(question: string) {
    if (disposed || busy || !question.trim()) return;
    question = question.slice(0, 1200);
    const current = ++run;
    original ||= question;
    input.value = question;
    controller = new AbortController();
    const signal = controller.signal;
    setBusy(true);
    setStatus(state === 'available' ? 'Reading this section…' : 'Preparing on-device AI…');
    let timedOut = false;
    let timer: number | undefined;
    // This call precedes any await, preserving activation for a first model download.
    const sessionPromise = createSession(model, signal, (text) => {
      if (run === current && !signal.aborted) setStatus(text);
    });
    try {
      const [session, data] = await Promise.all([sessionPromise, corpus()]);
      signal.throwIfAborted();
      if (disposed || run !== current) return;
      // A first model download can take much longer than an answer. Keep it cancellable
      // without imposing the inference timeout on that browser-managed download.
      timer = window.setTimeout(() => { timedOut = true; controller?.abort(); }, 180000);
      state = 'available';
      note.textContent = 'On-device AI · Check the linked sources.';
      const candidates = retrieve(data, entry.topic, question);
      if (!candidates.some((p) => inTopic(p, entry.topic))) throw new Error('Section missing');
      setStatus('Connecting the explanation to the wiki…');
      const { reply, passages } = await explain(session, entry.topic, question, candidates, history, trail, signal);
      if (disposed || run !== current) return;
      history = [...history, { question, answer: reply.answer }].slice(-2);
      showReply(reply, passages, data, question);
      input.value = '';
      label.textContent = 'Still unclear? Ask a follow-up';
      setStatus(reply.status === 'answered' ? 'Explanation ready.' : 'Check the section or try a more specific question.');
    } catch {
      if (!disposed && run === current) {
        setStatus(timedOut ? 'This is taking too long. You can try again.' : signal.aborted
          ? 'Stopped. Your question is still here.'
          : 'Couldn’t prepare a reliable explanation. Try again, or keep reading the section.');
      }
    } finally {
      clearTimeout(timer);
      void sessionPromise.then((session) => session.destroy(), () => {});
      if (!disposed && run === current) setBusy(false);
    }
  }
  for (const choice of choices) {
    const option = button(choice.label);
    option.addEventListener('click', () => { void ask(choice.question); });
    options.append(option);
  }
  // The corpus is loaded only after someone opens a helper or explicitly follows its link.
  // A failed chunk is retryable on the next question.
  void corpus().catch(() => {});
  if (incoming) {
    setBusy(true);
    setStatus('Picking up the connection…');
    void corpus().then((data) => {
      if (disposed) return;
      setBusy(false);
      if (data.version !== incoming.version || !data.passages.some((p) => p.url === incoming.from)) {
        setStatus('The documentation changed. Ask a fresh question about this section.');
        return;
      }
      trail = incoming;
      original = incoming.original;
      intro.replaceChildren('Continuing from ');
      const back = element('a', '', incoming.fromTitle);
      back.href = incoming.from;
      intro.append(back, '. Here is how this section connects.');
      if (state === 'available') void ask(incoming.question);
      else {
        input.value = incoming.question;
        submit.disabled = false;
        setStatus('Press Ask to continue on this device.');
      }
    }).catch(() => { if (!disposed) { setBusy(false); setStatus('Couldn’t load the wiki context. Try asking again.'); } });
    card.scrollIntoView({ block: 'center', behavior: 'instant' });
    close.focus({ preventScroll: true });
  } else {
    if (card.getBoundingClientRect().top > window.innerHeight - 180) {
      card.scrollIntoView({ block: 'start', behavior: 'instant' });
    }
    options.querySelector('button')?.focus({ preventScroll: true });
  }
}

function historyPush(url: string) {
  window.history.pushState(null, '', url);
}
