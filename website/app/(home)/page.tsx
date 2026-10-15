import Link from 'next/link';
import { appDescription, appName, basePath } from '@/lib/shared';

const features = [
  {
    title: 'Define a sensor once, spawn hundreds',
    body: 'Templates and counts turn one sensor definition into a whole fleet.',
  },
  {
    title: 'Behaves like a real device',
    body: 'Every sensor has its own connection and can announce online and offline with a Last Will.',
  },
  {
    title: 'GUI and headless CLI',
    body: 'Edit and watch live in the GUI, or run the same project in CI.',
  },
];

export default function HomePage() {
  return (
    <div className="mx-auto flex w-full max-w-5xl flex-1 flex-col items-center gap-12 px-4 py-16 text-center">
      <header className="flex flex-col items-center gap-4">
        <h1 className="text-5xl font-bold tracking-tight">{appName}</h1>
        <p className="max-w-xl text-lg text-fd-muted-foreground">{appDescription}</p>
        <div className="mt-2 flex flex-wrap justify-center gap-3">
          <Link
            href="/docs/quick-start"
            className="rounded-full bg-fd-primary px-5 py-2.5 text-sm font-medium text-fd-primary-foreground transition-colors hover:bg-fd-primary/90"
          >
            Quick start
          </Link>
          <Link
            href="/docs/reference/project-file"
            className="rounded-full border bg-fd-secondary px-5 py-2.5 text-sm font-medium text-fd-secondary-foreground transition-colors hover:bg-fd-accent hover:text-fd-accent-foreground"
          >
            Reference
          </Link>
        </div>
      </header>

      <ul className="grid w-full gap-4 text-left sm:grid-cols-3">
        {features.map((feature) => (
          <li key={feature.title} className="rounded-xl border bg-fd-card p-5">
            <h2 className="mb-2 font-semibold">{feature.title}</h2>
            <p className="text-sm text-fd-muted-foreground">{feature.body}</p>
          </li>
        ))}
      </ul>

      {/* Plain <img>: Next.js does not add the base path to static asset URLs. */}
      <img
        src={`${basePath}/screenshots/window.png`}
        alt="The Senzor window"
        className="h-auto w-full max-w-4xl rounded-xl border shadow-lg"
      />
    </div>
  );
}
