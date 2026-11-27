import { DownloadSimpleIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { api, type MyFile } from "./api";
import { FileIcon } from "./FileIcon";
import { formatSize, type FileRef } from "./messages";
import { fileDate, filesOf, type FilesTab } from "./myFiles";
import { fullDateTime } from "./time";

const TABS: Array<{ id: FilesTab; label: string; empty: string }> = [
  { id: "hasil", label: "Hasil Bakase", empty: "Belum ada file buatan Bakase. Minta Bakase membuat Excel, Word, atau PowerPoint di chat." },
  { id: "upload", label: "Upload", empty: "Belum ada file yang kamu lampirkan di chat." },
];

function FileRow({
  file,
  active,
  onOpen,
  onOpenChat,
}: {
  file: MyFile;
  active: boolean;
  onOpen: () => void;
  onOpenChat: () => void;
}) {
  return (
    <li
      className={`doc-row file-row${active ? " active" : ""}`}
      onClick={(e) => {
        if (!(e.target as Element).closest("button, a")) onOpen();
      }}
    >
      <div className="doc-open">
        <FileIcon kind={file.kind} size={20} />
        <span className="doc-text">
          <button type="button" className="doc-title file-name" title={file.name} onClick={onOpen}>
            {file.name}
          </button>
          <span className="doc-sub">
            <button type="button" className="file-chat" title="Buka percakapannya" onClick={onOpenChat}>
              {file.conversation_title}
            </button>
            {" · "}
            <span title={fullDateTime(file.created_at)}>{fileDate(file.created_at)}</span>
            {" · "}
            {formatSize(file.size)}
          </span>
        </span>
      </div>
      <span />
      <a className="icon-btn doc-dl" href={`/api/files/${file.id}`} download aria-label={`Unduh ${file.name}`} title="Unduh">
        <DownloadSimpleIcon size={17} />
      </a>
    </li>
  );
}

/** Halaman File Saya: file buatan Bakase dan lampiran dari chat milik user sendiri. */
export function FilesPage({
  openId,
  onOpen,
  onOpenChat,
}: {
  openId: string | null;
  onOpen: (file: FileRef) => void;
  onOpenChat: (conversationId: string) => void;
}) {
  const [tab, setTab] = useState<FilesTab>("hasil");
  const [files, setFiles] = useState<MyFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    let active = true;
    setError(null);
    api<MyFile[]>("/api/files")
      .then((list) => active && setFiles(list))
      .catch((e: Error) => active && setError(e.message));
    return () => {
      active = false;
    };
  }, [retry]);

  const shown = files ? filesOf(files, tab) : [];
  const current = TABS.find((t) => t.id === tab)!;

  return (
    <div className="page docs-page files-page">
      <h2>File Saya</h2>
      <p className="docs-lead muted">File dari chat-mu sendiri. Hanya kamu yang bisa melihatnya.</p>
      <div className="search-tabs docs-tabs" role="tablist" aria-label="Jenis file">
        {TABS.map((t) => (
          <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} onClick={() => setTab(t.id)}>
            {t.label}
          </button>
        ))}
      </div>
      {error && (
        <p className="form-error">
          {error}{" "}
          <button type="button" className="text-btn" onClick={() => setRetry((r) => r + 1)}>
            Coba lagi
          </button>
        </p>
      )}
      <p className="docs-count" aria-live="polite">
        {files ? `${shown.length} file` : error ? "" : "Memuat…"}
      </p>
      <ul className="docs-list">
        {shown.map((f) => (
          <FileRow
            key={f.id}
            file={f}
            active={f.id === openId}
            onOpen={() => onOpen({ id: f.id, name: f.name, kind: f.kind, size: f.size })}
            onOpenChat={() => onOpenChat(f.conversation_id)}
          />
        ))}
      </ul>
      {files && shown.length === 0 && <p className="docs-empty">{current.empty}</p>}
    </div>
  );
}
