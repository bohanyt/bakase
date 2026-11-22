import { createContext } from "react";

/** Jawaban yang sedang mengalir. Streamdown menutup blok kode yang belum selesai, jadi
 * `isIncomplete` tidak cukup untuk tahu JSON grafik masih terpotong. */
export const AnswerRunning = createContext(false);
