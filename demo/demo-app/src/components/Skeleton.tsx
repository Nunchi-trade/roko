interface SkeletonProps {
  width?: string;
  height?: string;
  className?: string;
}

export default function Skeleton({ width = '100%', height = '20px', className }: SkeletonProps) {
  return (
    <>
      <div
        className={className}
        style={{
          width, height, borderRadius: 2,
          background: 'linear-gradient(90deg, rgba(100,80,90,.08) 25%, rgba(100,80,90,.16) 50%, rgba(100,80,90,.08) 75%)',
          backgroundSize: '200% 100%',
          animation: 'shimmer 1.5s ease-in-out infinite',
        }}
      />
      <style>{`@keyframes shimmer { 0%{background-position:200% 0} 100%{background-position:-200% 0} }`}</style>
    </>
  );
}
