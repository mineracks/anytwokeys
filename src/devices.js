// The signing devices a vault can hold, drawn in the house line style (the camera chooser's classes: c-body, c-fill,
// c-glass, c-line, c-beam — so they take the theme and currentColor). Shapes are generic silhouettes, no makers' marks.
//
// `offers` is what the DEVICE has (qr camera/display, sd card, usb, bluetooth, nfc); what this app can use of it today is
// `DEVICE_NOW` — the intersection is what the screen offers. Bluetooth is listed as the device's, not the app's: each
// maker's third-party protocol is checked before it is used (roadmap/anytwokeys/SIGNERS.md, step E).
const DEVICE_NOW = { qr: true, sd: true, paste: true, usb: false, ble: false };

const DEVICES = (() => {
  const g = (x, y, w, h, r = 1.5) => `<rect class="c-glass" x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}"/>`;
  const b = (x, y, w, h, r = 4) => `<rect class="c-body" x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}"/>`;
  const f = (x, y, w, h, r = 1) => `<rect class="c-fill" x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}"/>`;
  const dot = (cx, cy, r) => `<circle class="c-fill" cx="${cx}" cy="${cy}" r="${r}"/>`;
  const ring = (cx, cy, r) => `<circle class="c-glass" cx="${cx}" cy="${cy}" r="${r}"/>`;
  const lens = (cx, cy, r) => ring(cx, cy, r) + dot(cx, cy, r / 2.6);
  const grid = (x0, y0, cols, rows, dx, dy, w, h) => { let s = ''; for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) s += f(x0 + c * dx, y0 + r * dy, w, h, .8); return s; };
  const pads = (x0, y0, cols, rows, dx, dy, r) => { let s = ''; for (let j = 0; j < rows; j++) for (let i = 0; i < cols; i++) s += dot(x0 + i * dx, y0 + j * dy, r); return s; };
  const waves = (x, y) => `<path class="c-beam" d="M${x} ${y - 4}a5 5 0 0 1 0 8M${x + 4} ${y - 7}a9 9 0 0 1 0 14"/>`;
  const svg = (inner) => `<svg viewBox="0 0 72 56" role="img" aria-hidden="true">${inner}</svg>`;
  const list = [
    // ---- security keys: a card, plugged in, appears by itself ----
    { id: 'yubikey', name: 'YubiKey 5', maker: 'Yubico', group: 'card', offers: ['usb', 'nfc'],
      art: b(4, 24, 11, 10, 1.5) + b(14, 19, 48, 20, 4) + lens(40, 29, 6) + `<circle class="c-line" cx="55" cy="29" r="2.4"/>` },
    // a Nitrokey 3: a flat wedge, widest at the plug and tapering to a lanyard slot, the whole top face being the touch
    { id: 'nitrokey3', name: 'Nitrokey 3', maker: 'Nitrokey', group: 'card', offers: ['usb', 'nfc'],
      art: b(3, 24.5, 9, 7, 2.5) + `<path class="c-body" d="M14 14h2l48 5.2q4 .5 4 4.3v9q0 3.8-4 4.3L16 42h-2q-4 0-4-4V18q0-4 4-4z"/>`
        + `<rect class="c-glass" x="57" y="25.5" width="5" height="5" rx="1.2"/>` + ring(36, 28, 4.5) + `<path class="c-beam" d="M44 22.5l1.6-2.4M46 27h3M44.5 31.5l1.6 2.4"/>` + f(14.5, 19, 3, 2.2, 1) },
    // ---- air-gapped: codes on a screen, or a file on a card ----
    { id: 'seedsigner', name: 'SeedSigner', maker: 'SeedSigner', group: 'airgap', offers: ['qr'],
      how: 'Seeds › (the seed) › Export Xpub › Multisig › Native Segwit › Sparrow. Point this camera at the code it shows.',
      art: b(6, 10, 60, 36, 3) + g(26, 15, 22, 24) + ring(16, 28, 5.5) + dot(16, 28, 2) + dot(57, 20, 2) + dot(57, 28, 2) + dot(57, 36, 2) + lens(36, 7, 2.6) },
    { id: 'coldcard-mk4', name: 'Coldcard Mk4', maker: 'Coinkite', group: 'airgap', offers: ['sd', 'usb', 'nfc'],
      how: 'Settings › Multisig Wallets › Export XPUB. It writes a ccxp-….json file to the SD card; open that file here.',
      art: b(22, 3, 28, 50, 4) + g(26, 8, 20, 13) + grid(27, 25, 3, 4, 6.8, 6.4, 4.4, 3.4) + `<path class="c-line" d="M50 14h4"/>` },
    { id: 'coldcard-q', name: 'Coldcard Q', maker: 'Coinkite', group: 'airgap', offers: ['qr', 'sd', 'usb', 'nfc'],
      how: 'Settings › Multisig Wallets › Export XPUB, to the SD card. Open the file here.',
      art: b(5, 6, 62, 44, 5) + g(11, 10, 50, 17) + grid(11.5, 31, 10, 3, 5, 5.2, 3.4, 3) + lens(61, 3.5, 2) },
    { id: 'jade', name: 'Jade', maker: 'Blockstream', group: 'airgap', offers: ['qr', 'usb', 'ble'],
      how: 'In QR mode: Options › Wallet › Export Xpub, choose multisig, native segwit. Point this camera at its code.',
      art: b(15, 10, 40, 36, 8) + g(21, 16, 28, 18) + b(55, 21, 5, 13, 2.5) + f(26, 39, 7, 2.6) + f(38, 39, 7, 2.6) + waves(64, 12) },
    { id: 'jade-plus', name: 'Jade Plus', maker: 'Blockstream', group: 'airgap', offers: ['qr', 'usb', 'ble'],
      how: 'In QR mode: Options › Wallet › Export Xpub, choose multisig, native segwit. Point this camera at its code.',
      art: b(11, 9, 48, 38, 9) + g(17, 15, 36, 20) + f(22, 40, 8, 2.6) + f(40, 40, 8, 2.6) + lens(35, 41.3, 2.2) + waves(66, 12) },
    { id: 'keystone3', name: 'Keystone 3', maker: 'Keystone', group: 'airgap', offers: ['qr', 'sd'],
      how: 'Its multisig wallet menu shows the extended public key as a code. Point this camera at it.',
      art: b(18, 2, 36, 52, 6) + g(21, 7, 30, 38, 2) + ring(36, 49.5, 2.6) },
    { id: 'passport', name: 'Passport', maker: 'Foundation', group: 'airgap', offers: ['qr', 'sd'],
      how: 'Connect Wallet › Sparrow › Multisig, by QR code or to the microSD card.',
      art: b(20, 2, 32, 52, 8) + g(24, 7, 24, 20, 2) + pads(28, 32, 3, 4, 8, 5.6, 1.9) },
    { id: 'krux', name: 'Krux', maker: 'Krux (open source)', group: 'airgap', offers: ['qr', 'sd'],
      how: 'Load the seed, then Extended Public Key › the multisig key. Point this camera at its code.',
      art: b(22, 5, 28, 46, 6) + lens(36, 17, 6.5) + g(27, 29, 18, 16) + `<path class="c-line" d="M50 14v8"/>` },
    { id: 'specter-diy', name: 'Specter DIY', maker: 'Specter (open source)', group: 'airgap', offers: ['qr', 'sd', 'usb'],
      how: 'Master public keys › the multisig (native segwit) key. Point this camera at its code.',
      art: b(4, 6, 50, 44, 3) + g(9, 10, 40, 36) + b(55, 17, 9, 22, 2) + `<path class="c-beam" d="M64 22l6-3M64 28h6M64 34l6 3"/>` },
    // ---- by cable (and some by Bluetooth): until the app speaks to them, their own app shows the key to paste ----
    { id: 'trezor-safe3', name: 'Trezor Safe 3', maker: 'Trezor', group: 'usb', offers: ['usb'],
      art: b(20, 8, 32, 40, 6) + g(24, 13, 24, 17) + f(24, 37, 9, 3) + f(39, 37, 9, 3) + `<path class="c-line" d="M32 51h8"/>` },
    { id: 'trezor-safe5', name: 'Trezor Safe 5', maker: 'Trezor', group: 'usb', offers: ['usb'],
      art: b(20, 3, 32, 50, 7) + g(23.5, 7, 25, 42, 4) },
    { id: 'trezor-safe7', name: 'Trezor Safe 7', maker: 'Trezor', group: 'usb', offers: ['usb', 'ble'],
      art: b(18, 2, 36, 52, 8) + g(21.5, 6, 29, 44, 5) + waves(60, 12) },
    { id: 'trezor-t', name: 'Trezor Model T', maker: 'Trezor', group: 'usb', offers: ['usb'],
      art: b(15, 5, 42, 46, 6) + g(19, 9, 34, 32, 2) + `<path class="c-line" d="M28 46h16"/>` },
    { id: 'ledger-nanosp', name: 'Ledger Nano S Plus', maker: 'Ledger', group: 'usb', offers: ['usb'],
      art: b(6, 22, 54, 14, 3) + g(22, 25, 20, 8, 1) + f(14, 19.6, 6, 2.4) + f(46, 19.6, 6, 2.4) + `<path class="c-line" d="M4 20h54a6 9 0 0 1 0 18H4"/>` + ring(58, 29, 3.4) },
    { id: 'ledger-nanox', name: 'Ledger Nano X', maker: 'Ledger', group: 'usb', offers: ['usb', 'ble'],
      art: b(4, 20, 56, 18, 4) + g(22, 24, 22, 10, 1) + ring(13, 29, 4) + ring(52, 29, 4) + `<path class="c-line" d="M2 18h58a7 11 0 0 1 0 22H2"/>` + waves(66, 12) },
    { id: 'ledger-stax', name: 'Ledger Stax', maker: 'Ledger', group: 'usb', offers: ['usb', 'ble', 'nfc'],
      art: b(14, 4, 40, 48, 3) + `<path class="c-line" d="M19 4v48"/>` + g(22, 8, 28, 40, 1.5) + waves(61, 12) },
    { id: 'ledger-flex', name: 'Ledger Flex', maker: 'Ledger', group: 'usb', offers: ['usb', 'ble', 'nfc'],
      art: b(15, 4, 40, 48, 7) + g(19, 8, 32, 40, 3) + f(55, 14, 2.2, 7) + waves(62, 12) },
    { id: 'bitbox02', name: 'BitBox02', maker: 'BitBox', group: 'usb', offers: ['usb'],
      art: b(8, 21, 52, 14, 3) + g(21, 24, 26, 8, 1) + f(12, 17.6, 44, 1.8, .9) + f(12, 36.6, 44, 1.8, .9) + b(60, 25, 7, 6, 2) },
    { id: 'onekey-pro', name: 'OneKey Pro', maker: 'OneKey', group: 'usb', offers: ['usb', 'ble', 'qr'],
      art: b(18, 2, 36, 52, 8) + g(21.5, 6, 29, 44, 5) + lens(36, 10.5, 2.2) + waves(60, 12) },
    { id: 'keepkey', name: 'KeepKey', maker: 'KeepKey', group: 'usb', offers: ['usb'],
      art: b(3, 15, 66, 26, 4) + g(9, 20, 40, 16, 1) + lens(59, 28, 4.5) },
    // ---- anything else: a key another wallet shows ----
    { id: 'other', name: 'Another signer', maker: '', group: 'other', offers: [],
      how: 'Any signer or wallet that shows its multisig key as [fingerprint/48h/0h/0h/2h]xpub…, a Coldcard-style JSON file, or a UR code.',
      art: `<path class="c-body" d="M20 4h24l8 8v40H20z"/><path class="c-line" d="M44 4v8h8M26 22h20M26 29h20M26 36h14"/><path class="c-line" d="M26 45h8"/>` },
  ];
  for (const d of list) d.svg = svg(d.art);
  return list;
})();
const deviceOf = (id) => DEVICES.find(d => d.id === id) || DEVICES.find(d => d.id === 'other');
// what this app can do with a device today, in the words the screen uses
const DEVICE_WORDS = { qr: 'QR', sd: 'SD card', usb: 'USB', ble: 'Bluetooth', nfc: 'NFC' };
