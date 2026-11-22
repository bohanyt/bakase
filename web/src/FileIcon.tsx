import {
  FileIcon as GenericFileIcon,
  FilePdfIcon,
  FileTextIcon,
  ImageIcon,
  MicrosoftExcelLogoIcon,
  MicrosoftPowerpointLogoIcon,
  MicrosoftWordLogoIcon,
} from "@phosphor-icons/react";

/** Ikon file sesuai jenisnya: Excel hijau, Word biru, PowerPoint oranye, PDF merah. */
export function FileIcon({ kind, size }: { kind: string; size: number }) {
  switch (kind) {
    case "xlsx":
    case "xls":
    case "xlsm":
      return <MicrosoftExcelLogoIcon size={size} weight="fill" className="file-icon xls" />;
    case "docx":
    case "doc":
      return <MicrosoftWordLogoIcon size={size} weight="fill" className="file-icon doc" />;
    case "pptx":
    case "ppt":
      return <MicrosoftPowerpointLogoIcon size={size} weight="fill" className="file-icon ppt" />;
    case "pdf":
      return <FilePdfIcon size={size} weight="fill" className="file-icon pdf" />;
    case "txt":
    case "md":
    case "csv":
      return <FileTextIcon size={size} className="file-icon" />;
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "webp":
      return <ImageIcon size={size} className="file-icon" />;
    default:
      return <GenericFileIcon size={size} className="file-icon" />;
  }
}
