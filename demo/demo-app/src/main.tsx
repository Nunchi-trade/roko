import { StrictMode, Suspense, lazy } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, Routes, Route } from 'react-router';
import ErrorBoundary from './components/ErrorBoundary';
import AppShell from './components/AppShell';
import { WorkspaceProvider } from './hooks/useWorkspace';
import { RokoConfigProvider } from './hooks/useRokoConfig';
import { ToastProvider } from './components/Toast';
import { EventStreamProvider } from './contexts/EventStreamContext';
import { bootstrapTransport } from './app/bootstrap';
import './styles/rosedust.css';
import './styles/typography.css';
import './styles/animations.css';
import './styles/motion.css';
import './styles/interactions.css';
import './styles/loading.css';
import './styles/ambient.css';
import './styles/scrollbar.css';
import './styles/focus.css';
import './styles/gradient-borders.css';

const Landing = lazy(() => import('./pages/Landing'));
const DashboardLayout = lazy(() => import('./pages/dashboard/Layout'));
const CostDashboard = lazy(() => import('./pages/dashboard/CostDashboard'));
const AgentFleet = lazy(() => import('./pages/dashboard/AgentFleet'));
const KnowledgeGraph = lazy(() => import('./pages/dashboard/KnowledgeGraph'));
const IntegrityView = lazy(() => import('./pages/dashboard/IntegrityView'));
const CascadeRouter = lazy(() => import('./pages/dashboard/CascadeRouter'));
const KnowledgeEntries = lazy(() => import('./pages/dashboard/KnowledgeEntries'));
const DreamsView = lazy(() => import('./pages/dashboard/DreamsView'));
const IsfrPage = lazy(() => import('./pages/dashboard/IsfrPage'));
const Terminal = lazy(() => import('./pages/Terminal'));
const Builder = lazy(() => import('./pages/Builder'));
const Explorer = lazy(() => import('./pages/Explorer/index'));
const Bench = lazy(() => import('./pages/Bench'));
const BenchRunDetail = lazy(() => import('./pages/BenchRunDetail'));
const BenchCompare = lazy(() => import('./pages/BenchCompare'));
const Settings = lazy(() => import('./pages/Settings'));
const SharePage = lazy(() => import('./pages/Share'));

function RouteLoading() {
  return (
    <div className="route-loading progressive-reveal">
      {/* Fake nav row */}
      <div className="route-loading__nav">
        <div className="skeleton route-loading__nav-pill" />
        <div className="skeleton route-loading__nav-pill" style={{ width: 96 }} />
        <div className="skeleton route-loading__nav-pill" style={{ width: 56 }} />
      </div>

      {/* Fake header */}
      <div className="route-loading__header">
        <div className="skeleton skeleton-circle" />
        <div className="skeleton skeleton-title" />
      </div>

      {/* Fake mosaic stats */}
      <div className="route-loading__mosaic">
        {Array.from({ length: 4 }, (_, i) => (
          <div key={i} className="route-loading__mosaic-cell">
            <div className="skeleton skeleton-text" style={{ width: '50%' }} />
            <div className="skeleton skeleton-title" style={{ width: '70%' }} />
          </div>
        ))}
      </div>

      {/* Fake body lines */}
      <div className="route-loading__body">
        <div className="skeleton-card skeleton" />
        <div className="route-loading__row">
          <div className="skeleton skeleton-text" style={{ width: '40%' }} />
          <div className="skeleton skeleton-text" style={{ width: '25%' }} />
        </div>
        <div className="skeleton skeleton-text" style={{ width: '80%' }} />
        <div className="skeleton skeleton-text" style={{ width: '55%' }} />
      </div>
    </div>
  );
}

// Initialize transport layer before React render.
// Cleanup is stored at module scope for HMR teardown if needed.
void bootstrapTransport();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <BrowserRouter>
      <ErrorBoundary>
        <WorkspaceProvider>
        <RokoConfigProvider>
        <EventStreamProvider>
        <ToastProvider>
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
                <Route path="isfr" element={<IsfrPage />} />
              </Route>
              <Route path="demo" element={null} />
              <Route path="terminal" element={<Terminal />} />
              <Route path="builder" element={<Builder />} />
              <Route path="explorer" element={<Explorer />} />
              <Route path="settings" element={<Settings />} />
              <Route path="bench" element={<Bench />} />
              <Route path="bench/run/:id" element={<BenchRunDetail />} />
              <Route path="bench/compare" element={<BenchCompare />} />
              <Route path="share/:token" element={<SharePage />} />
              <Route path="share" element={<SharePage />} />
            </Route>
          </Routes>
        </Suspense>
        </ToastProvider>
        </EventStreamProvider>
        </RokoConfigProvider>
        </WorkspaceProvider>
      </ErrorBoundary>
    </BrowserRouter>
  </StrictMode>,
);
