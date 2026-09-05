/* Shared DOM behavior. Views own constraints and redraws, not pointer mechanics. */
const ExplorerRuntime = (() => {
  function bindResizer(handle, {read, write, initial, sign, cancelInspection = () => {}}) {
    let gesture = null;
    handle.setAttribute('aria-valuenow', read());
    handle.addEventListener('pointerdown', event => {
      if (event.button !== 0) return;
      cancelInspection();
      gesture = {x: event.clientX, width: read(), pointerId: event.pointerId};
      handle.setPointerCapture(event.pointerId);
      document.body.classList.add('resizing');
      event.preventDefault();
    });
    handle.addEventListener('pointermove', event => {
      if (gesture?.pointerId === event.pointerId) write(gesture.width + (event.clientX - gesture.x) * sign);
    });
    const finish = event => {
      if (!gesture || gesture.pointerId !== event.pointerId) return;
      gesture = null;
      document.body.classList.remove('resizing');
      if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
    };
    for (const name of ['pointerup', 'pointercancel', 'lostpointercapture']) handle.addEventListener(name, finish);
    handle.addEventListener('keydown', event => {
      if (!['ArrowLeft', 'ArrowRight', 'Home'].includes(event.key)) return;
      event.preventDefault();
      cancelInspection();
      write(event.key === 'Home' ? initial : read() + (event.key === 'ArrowRight' ? 24 : -24) * sign);
    });
  }
  function bindDepth(slider, input, change) {
    const apply = value => {
      const parsed = Number.parseInt(value, 10);
      const depth = Math.max(Number(input.min), Math.min(Number(input.max), Number.isFinite(parsed) ? parsed : Number(input.min)));
      slider.value = input.value = depth;
      change(depth);
    };
    slider.addEventListener('input', () => apply(slider.value));
    input.addEventListener('change', () => apply(input.value));
    input.addEventListener('keydown', event => {if (event.key === 'Enter') {event.preventDefault(); apply(input.value);}});
  }
  function paneToggle(button, open, label) {
    button.setAttribute('aria-pressed', open);
    button.textContent = `${open ? 'Hide' : 'Show'} ${label}`;
  }
  return {bindResizer, bindDepth, paneToggle};
})();
if (typeof module !== 'undefined') module.exports = ExplorerRuntime;
