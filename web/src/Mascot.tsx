import { mascotAssets, mascotSizes } from "./mascotAssets";

type Props = {
  asset: keyof typeof mascotAssets;
  placement?: keyof typeof mascotSizes;
  className?: string;
};

export function Mascot({ asset, placement = "status", className }: Props) {
  const size = mascotSizes[placement];
  const image: { src: string; poster?: string } | null = mascotAssets[asset];
  if (!size || !image) return null;
  const [width, height] = size;
  const { src, poster } = image;
  if (!poster) return <img className={className} src={src} width={width} height={height} alt="" />;

  return (
    <picture className={className}>
      <source media="(prefers-reduced-motion: reduce)" srcSet={poster} />
      <img className="mascot-image" src={src} width={width} height={height} alt="" />
    </picture>
  );
}
