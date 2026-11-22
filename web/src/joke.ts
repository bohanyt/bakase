/** Tombol "Joke on/off": pilihan ini berlaku untuk semua chat dan diingat di browser ini. */
const KEY = "app.joke";

let current = (() => {
  try {
    return localStorage.getItem(KEY) === "on";
  } catch {
    return false;
  }
})();

export function getJoke(): boolean {
  return current;
}

export function setJoke(on: boolean) {
  current = on;
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    /* penyimpanan browser tidak tersedia; pilihan tetap berlaku sampai halaman dimuat ulang */
  }
}
