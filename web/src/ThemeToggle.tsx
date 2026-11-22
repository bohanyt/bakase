import { useState } from "react";
import { currentTheme, setTheme, type Theme } from "./theme";
import "./theme-switch.css";

const PARTS = Array.from({ length: 11 }, (_, i) => i + 1);

/** Toggle geser kiri-kanan: matahari (terang) berubah jadi bulan (gelap). */
export function ThemeToggle() {
  const [theme, setState] = useState<Theme>(currentTheme);
  return (
    <label className="theme-switch" title={theme === "dark" ? "Ganti ke mode terang" : "Ganti ke mode gelap"}>
      <input
        id="theme-switch"
        className="theme-switch__input"
        type="checkbox"
        role="switch"
        checked={theme === "dark"}
        onChange={(e) => {
          const next: Theme = e.target.checked ? "dark" : "light";
          setTheme(next);
          setState(next);
        }}
      />
      <span className="theme-switch__icon" aria-hidden>
        {PARTS.map((n) => (
          <span key={n} className={`theme-switch__part theme-switch__part--${n}`} />
        ))}
      </span>
      <span className="theme-switch__sr">Mode gelap</span>
    </label>
  );
}
