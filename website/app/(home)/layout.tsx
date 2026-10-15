import { HomeLayout } from 'fumadocs-ui/layouts/home';
import { baseOptions } from '@/lib/layout.shared';

export default function Layout({ children }: LayoutProps<'/'>) {
  // The docs layout lists the Introduction in its own sidebar, so only the home page needs this link.
  return (
    <HomeLayout {...baseOptions()} links={[{ text: 'Docs', url: '/docs' }]}>
      {children}
    </HomeLayout>
  );
}
