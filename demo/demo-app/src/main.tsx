import { StrictMode, Suspense, lazy } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, Routes, Route } from 'react-router';
import ErrorBoundary from './components/ErrorBoundary';
import { EventStreamProvider } from './contexts/EventStreamContext';
import AppShell from './components/AppShell';
import './styles/rosedust.css';

const Landing = lazy(() => import('./pages/Landing'));
const DashboardLayout = lazy(() => import('./pages/dashboard/Layout'));
const CostDashboard = lazy(() => import('./pages/dashboard/CostDashboard'));
const AgentFleet = lazy(() => import('./pages/dashboard/AgentFleet'));
const KnowledgeGraph = lazy(() => import('./pages/dashboard/KnowledgeGraph'));
const IntegrityView = lazy(() => import('./pages/dashboard/IntegrityView'));
const CascadeRouter = lazy(() => import('./pages/dashboard/CascadeRouter'));
const KnowledgeEntries = lazy(() => import('./pages/dashboard/KnowledgeEntries'));
const DreamsView = lazy(() => import('./pages/dashboard/DreamsView'));
const Demo = lazy(() => import('./pages/Demo'));
const Terminal = lazy(() => import('./pages/Terminal'));
const Builder = lazy(() => import('./pages/Builder'));
const Explorer = lazy(() => import('./pages/Explorer'));
const Bench = lazy(() => import('./pages/Bench'));
const BenchRunDetail = lazy(() => import('./pages/BenchRunDetail'));
const BenchCompare = lazy(() => import('./pages/BenchCompare'));
const Settings = lazy(() => import('./pages/Settings'));
const SharePage = lazy(() => import('./pages/Share'));

function RouteLoading() {
  return (
    <div
      style={{
        minHeight: '50vh',
        display: 'grid',
        placeItems: 'center',
        color: 'var(--text-dim)',
        fontFamily: 'var(--mono)',
        fontSize: 14,
        letterSpacing: '.08em',
        textTransform: 'uppercase',
      }}
    >
      Loading view
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <BrowserRouter>
      <EventStreamProvider>
      <ErrorBoundary>
        <Suspense fallback={<RouteLoading />}>
          <Routes>
            <Route element={<AppShell />}>
              <Route index element={<Landing />} />
              <Route path="dashboard" element={<DashboardLayout />}>
                <Route index element={<CostDashboard />} />
                <Route path="fleet" element={<AgentFleet />} />
                <Route path="knowledge" element={<KnowledgeGraph />} />
                <Route path="integrity" element={<IntegrityView />} />
                <Route path="entries" element={<KnowledgeEntries />} />
                <Route path="routing" element={<CascadeRouter />} />
                <Route path="dreams" element={<DreamsView />} />
              </Route>
              <Route path="demo" element={<Demo />} />
              <Route path="terminal" element={<Terminal />} />
              <Route path="builder" element={<Builder />} />
              <Route path="explorer" element={<Explorer />} />
              <Route path="settings" element={<Settings />} />
              <Route path="bench" element={<Bench />} />
              <Route path="bench/run/:id" element={<BenchRunDetail />} />
              <Route path="bench/compare" element={<BenchCompare />} />
              <Route path="share/:token" element={<SharePage />} />
            </Route>
          </Routes>
        </Suspense>
      </ErrorBoundary>
      </EventStreamProvider>
    </BrowserRouter>
  </StrictMode>,
);
