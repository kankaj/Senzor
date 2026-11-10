import Link from 'next/link';
import { VersionRedirect } from '@/components/version-redirect';

export default function NotFound() {
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <VersionRedirect />
      <h1 className="text-2xl font-semibold">Page not found</h1>
      <p className="text-fd-muted-foreground">This page does not exist, or not in this version of the docs.</p>
      <Link href="/docs" className="underline">
        Go to the documentation
      </Link>
    </main>
  );
}
