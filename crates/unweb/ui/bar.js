// Unweb's bar. It only talks to the app through its own commands, which
// steer this window's strangers and nothing else. With several strangers
// side by side, the address goes to all of them, and Back, Forward and
// Reload act on all of them.
(function () {
  'use strict';
  var invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
  var address = document.getElementById('address');
  var note = document.getElementById('note');
  var back = document.getElementById('back');
  var forward = document.getElementById('forward');
  var side = document.getElementById('side');
  var setups = document.getElementById('setups');
  var names = [];
  // 'address', or 'name' while a setup is being named in the same field.
  var mode = 'address';
  // Set when the person types; the page's address doesn't overwrite what
  // they are typing until they press Enter or Escape.
  var typing = false;
  var shown = '';

  function call(name, args) {
    note.textContent = '';
    note.className = '';
    if (!invoke) return Promise.resolve(false);
    return invoke(name, args || {}).then(function () { return true; }, function (e) {
      note.textContent = String(e);
      return false;
    });
  }

  function say(text) { note.textContent = text; note.className = 'ok'; }

  function fillSetups(list) {
    var same = list.length === names.length && list.every(function (n, i) { return n === names[i]; });
    if (same && setups.options.length > 1) return;
    names = list.slice();
    while (setups.firstChild) setups.removeChild(setups.firstChild);
    function option(parent, value, text) {
      var o = document.createElement('option');
      o.value = value;
      o.textContent = text;
      parent.appendChild(o);
    }
    option(setups, '', 'Setups');
    option(setups, 'save', 'Save this window as a setup…');
    if (names.length) {
      var open = document.createElement('optgroup');
      open.label = 'Open in a new window';
      var del = document.createElement('optgroup');
      del.label = 'Delete';
      names.forEach(function (n, i) {
        option(open, 'open:' + i, n);
        option(del, 'del:' + i, n);
      });
      setups.appendChild(open);
      setups.appendChild(del);
    }
    setups.value = '';
  }

  function nameMode(on) {
    mode = on ? 'name' : 'address';
    typing = on;
    address.value = on ? '' : shown;
    address.placeholder = on ? 'Name this setup, then press Enter (Escape to cancel)' : address.dataset.placeholder || 'Type a web address';
    address.setAttribute('aria-label', on ? 'Setup name' : 'Web address');
    if (on) address.focus();
  }

  window.unwebShow = function (state) {
    shown = state.url || '';
    back.disabled = !state.back;
    forward.disabled = !state.forward;
    if (typeof state.tiles === 'number') {
      side.disabled = state.tiles >= state.max;
      side.title = side.disabled
        ? 'This window has as many strangers as it can show'
        : 'Add a stranger to this window, beside the others, with a store of its own';
      address.dataset.placeholder = state.tiles > 1
        ? 'Type a web address for all ' + state.tiles + ' strangers'
        : 'Type a web address';
      if (mode === 'address') address.placeholder = address.dataset.placeholder;
    }
    if (state.setups) fillSetups(state.setups);
    if (!typing) {
      address.value = shown;
      if (document.activeElement === address) address.select();
    }
  };

  document.getElementById('bar').addEventListener('submit', function (e) {
    e.preventDefault();
    if (mode === 'name') {
      var name = address.value;
      nameMode(false);
      call('save_setup', { name: name }).then(function (ok) { if (ok) say('Saved setup “' + name.trim() + '”'); });
      address.blur();
      return;
    }
    typing = false;
    call('go', { text: address.value });
    address.blur();
  });
  setups.addEventListener('change', function () {
    var v = setups.value;
    setups.value = '';
    if (v === 'save') { nameMode(true); return; }
    var m = /^(open|del):(\d+)$/.exec(v);
    if (!m || names[+m[2]] === undefined) return;
    var name = names[+m[2]];
    if (m[1] === 'open') {
      call('open_setup', { name: name });
    } else {
      call('delete_setup', { name: name }).then(function (ok) { if (ok) say('Deleted setup “' + name + '”'); });
    }
  });
  document.getElementById('compare').addEventListener('click', function () {
    call('compare');
  });
  address.addEventListener('input', function () { typing = true; note.textContent = ''; });
  address.addEventListener('focus', function () { address.select(); });
  address.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      if (mode === 'name') { nameMode(false); address.blur(); return; }
      typing = false; address.value = shown; address.select(); note.textContent = '';
    }
  });
  back.addEventListener('click', function () { call('back'); });
  forward.addEventListener('click', function () { call('forward'); });
  document.getElementById('reload').addEventListener('click', function () { call('reload'); });
  side.addEventListener('click', function () { call('side_by_side'); });
  document.getElementById('new').addEventListener('click', function () { call('new_stranger'); });
  if (!address.value) address.focus();
})();
