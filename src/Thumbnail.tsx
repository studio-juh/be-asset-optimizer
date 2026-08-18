import { useEffect, useRef, useState, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";

type Position = { left: number; top: number };

export default function Thumbnail({ path, alt, fallbackLabel }: { path: string; alt: string; fallbackLabel: string }) {
  const target = useRef<HTMLSpanElement>(null);
  const [thumbnail, setThumbnail] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [position, setPosition] = useState<Position | null>(null);

  useEffect(() => {
    const element = target.current;
    if (!element) return;
    let cancelled = false;
    const observer = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      observer.disconnect();
      void invoke<string>("create_preview", { path, size: 72 }).then((data) => { if (!cancelled) setThumbnail(data); }).catch(() => undefined);
    }, { rootMargin: "160px" });
    observer.observe(element);
    return () => { cancelled = true; observer.disconnect(); };
  }, [path]);

  const show = (event: MouseEvent<HTMLSpanElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const size = 220;
    setPosition({
      left: Math.min(rect.right + 10, window.innerWidth - size - 10),
      top: Math.min(Math.max(10, rect.top - (size - rect.height) / 2), window.innerHeight - size - 10),
    });
    if (!expanded) void invoke<string>("create_preview", { path, size: 384 }).then(setExpanded).catch(() => undefined);
  };

  return <>
    <span ref={target} className="thumbnail" onMouseEnter={show} onMouseLeave={() => setPosition(null)}>{thumbnail ? <img src={thumbnail} alt="" /> : <span className="thumbnail-loading">{fallbackLabel}</span>}</span>
    {position && createPortal(<div className="preview-overlay" style={position}>{expanded || thumbnail ? <img src={expanded || thumbnail || ""} alt={alt} /> : <span>Loading…</span>}</div>, document.body)}
  </>;
}
