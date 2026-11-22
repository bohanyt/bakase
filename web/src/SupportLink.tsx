import { WhatsappLogoIcon } from "@phosphor-icons/react";

const MESSAGE = "Halo IT, saya ada masalah di Bakase: ";

/** Tautan bantuan ke WhatsApp IT. Tidak tampil kalau nomornya belum diatur di server. */
export function SupportLink({ whatsapp }: { whatsapp?: string | null }) {
  if (!whatsapp) return null;
  const href = `https://wa.me/${whatsapp}?text=${encodeURIComponent(MESSAGE)}`;
  return (
    <a
      className="support-link"
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      title="Chat WhatsApp dengan IT"
      aria-label="Ada masalah? Hubungi IT lewat WhatsApp"
    >
      <WhatsappLogoIcon size={18} weight="fill" />
      <span>
        Ada masalah? <strong>Hubungi IT</strong>
      </span>
    </a>
  );
}
