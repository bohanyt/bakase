export const COPY = {
  redirect: ["Mengalihkan ke Microsoft…", "Siapkan akun Microsoft kantor."],
  wrong: ["Gunakan akun kantor", "Akun yang dipilih bukan akun Microsoft kantor APP.", "Pilih akun lain"],
  access: ["Akses belum tersedia", "Akun kantor kamu belum ada di daftar akses Bakase. Hubungi tim IT untuk meminta akses."],
  unavailable: ["Microsoft sedang tidak bisa dihubungi", "Tunggu sebentar, lalu coba lagi.", "Coba lagi"],
  canceled: ["Proses masuk dibatalkan", "Kamu bisa mencoba lagi saat siap.", "Coba lagi"],
  expired: ["Waktu masuk habis", "Kembali ke halaman masuk dan mulai lagi.", "Coba lagi"],
  invalid: ["Proses masuk tidak dapat diverifikasi", "Mulai lagi dari halaman masuk Bakase.", "Coba lagi"],
  disabled: ["Akun dinonaktifkan", "Hubungi tim IT untuk meminta bantuan."],
  linked: ["Akun sudah tertaut", "Email kantor ini sudah tertaut ke akun Microsoft lain. Hubungi tim IT untuk meminta bantuan."],
  pilot: ["Akses uji coba belum tersedia", "Akun kantor ini belum terdaftar untuk uji coba masuk dengan Microsoft.", "Pilih akun lain"],
  waiting: ["Menunggu Microsoft", "Selesaikan proses masuk di tab baru. Bakase akan melanjutkan otomatis."],
  done: ["Kamu sudah masuk", "Panel Bakase siap digunakan."],
} as const;

export type Status = keyof typeof COPY;

export function microsoftParams() {
  const query = new URLSearchParams(location.search);
  return {
    pilot: query.get("microsoft") === "pilot",
    tab: query.get("ms_tab") === "1",
    result: query.get("ms"),
  };
}

export function microsoftResult(raw: string | null): Status | null {
  return raw && raw !== "done" && raw !== "waiting" && raw !== "redirect" && Object.hasOwn(COPY, raw)
    ? raw as Status : null;
}
