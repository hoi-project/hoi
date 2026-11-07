// Relay page: a demo of the one switch. Turning it on shows the notice first, the way the desktop
// app does; the knob moves on a spring.
import { animate, createSpring } from 'animejs';
import { $, announce, reduce } from '../common';

const sw = $<HTMLButtonElement>('#relaySw');
const knob = sw ? $('i', sw) : null;
const notice = $('#notice');
const yes = $<HTMLButtonElement>('#noticeOn');
const no = $<HTMLButtonElement>('#noticeOff');
const pill = $('#relayPill');
const text = $('#relayText');
const helped = $('#statHelped');

function tick(el: HTMLElement, to: number): void {
  if (reduce) { el.textContent = String(to); return; }
  const o = { n: Number(el.textContent) || 0 };
  animate(o, { n: to, duration: 700, ease: 'outExpo', onUpdate: () => { el.textContent = String(Math.round(o.n)); } });
}

function set(on: boolean): void {
  if (!sw || !knob) return;
  if (sw.getAttribute('aria-checked') === String(on)) return;
  sw.setAttribute('aria-checked', String(on));
  if (!reduce) animate(knob, { translateX: on ? [0, 22] : [22, 0], ease: createSpring({ stiffness: 320, damping: 14 }) });
  pill?.classList.toggle('on', on);
  if (text) text.textContent = on ? 'relaying for 17 friends' : 'relaying is off';
  if (helped) tick(helped, on ? 17 : 0);
  announce(on ? 'Relay on. Friends can reach the network through you.' : 'Relay off.');
}

if (sw && notice && yes && no) {
  sw.addEventListener('click', () => {
    if (sw.getAttribute('aria-checked') === 'true') { set(false); return; }
    // Off → on goes through the notice, every time.
    notice.scrollIntoView({ block: 'nearest', behavior: reduce ? 'auto' : 'smooth' });
    if (!reduce) animate(notice, { scale: [1, 1.025, 1], duration: 500, ease: 'outBack(2)' });
    yes.focus({ preventScroll: true });
  });
  yes.addEventListener('click', () => { set(true); sw.focus(); });
  no.addEventListener('click', () => {
    if (sw.getAttribute('aria-checked') === 'true') set(false);
    else announce('Okay. Relaying stays off.');
    sw.focus();
  });
}
