// Unweb's bar. It only talks to the app through its own commands, which
// steer this window's page and nothing else.
(function () {
  'use strict';
  var invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
  var address = document.getElementById('address');
  var note = document.getElementById('note');
  var back = document.getElementById('back');
  var forward = document.getElementById('forward');
  // Set when the person types; the page's address doesn't overwrite what
  // they are typing until they press Enter or Escape.
  var typing = false;
  var shown = '';

  function call(name, args) {
    note.textContent = '';
    if (!invoke) return Promise.resolve();
    return invoke(name, args || {}).catch(function (e) {
      note.textContent = String(e);
    });
  }

  window.unwebShow = function (state) {
    shown = state.url || '';
    back.disabled = !state.back;
    forward.disabled = !state.forward;
    if (!typing) {
      address.value = shown;
      if (document.activeElement === address) address.select();
    }
  };

  document.getElementById('bar').addEventListener('submit', function (e) {
    e.preventDefault();
    typing = false;
    call('go', { text: address.value });
    address.blur();
  });
  address.addEventListener('input', function () { typing = true; note.textContent = ''; });
  address.addEventListener('focus', function () { address.select(); });
  address.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') { typing = false; address.value = shown; address.select(); note.textContent = ''; }
  });
  back.addEventListener('click', function () { call('back'); });
  forward.addEventListener('click', function () { call('forward'); });
  document.getElementById('reload').addEventListener('click', function () { call('reload'); });
  document.getElementById('new').addEventListener('click', function () { call('new_stranger'); });
  if (!address.value) address.focus();
})();
