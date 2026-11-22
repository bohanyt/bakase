import { createContext } from "react";
import type { QuoteDraft } from "./quote";

/** Menambah kutipan ke kotak tanya. Mengembalikan pesan error, atau null kalau berhasil.
 * Disediakan oleh Shell. */
export const CiteContext = createContext<(draft: QuoteDraft) => string | null>(() => null);
