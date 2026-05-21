type SectionEyebrowProps = {
  children: React.ReactNode;
  className?: string;
  as?: "p" | "span" | "div";
};

export function SectionEyebrow({
  children,
  className,
  as: Tag = "p",
}: SectionEyebrowProps) {
  return (
    <Tag
      className={[
        "font-mono text-eyebrow uppercase tracking-widest text-ink-subtle",
        className ?? "",
      ].join(" ")}
    >
      {children}
    </Tag>
  );
}
