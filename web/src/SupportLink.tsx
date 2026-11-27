import { WhatsappLogoIcon } from "@phosphor-icons/react";

const MESSAGE = "Halo IT, saya ada masalah di Bakase: ";

/** Tautan bantuan ke WhatsApp IT. Tidak tampil kalau nomornya belum diatur di server. */
export function SupportLink({ whatsapp, menu = false }: { whatsapp?: string | null; menu?: boolean }) {
  if (!whatsapp) return null;
  return (
    <a
      className={menu ? undefined : "support-link"}
      href={`https://wa.me/${whatsapp}?text=${encodeURIComponent(MESSAGE)}`}
      target="_blank"
      rel="noopener noreferrer"
      title={menu ? undefined : "Chat WhatsApp dengan IT"}
      aria-label={menu ? undefined : "Ada masalah? Hubungi IT lewat WhatsApp"}
    >
      <WhatsappLogoIcon className={menu ? "help-whatsapp" : undefined} size={menu ? 20 : 18} weight="fill" />
      {menu ? <span>Hubungi IT<small>Lewat WhatsApp</small></span> : <span>
        Ada masalah? <strong>Hubungi IT</strong>
      </span>}
    </a>
  );
}
