import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, Routes, Route } from 'react-router';
import ErrorBoundary from './components/ErrorBoundary';
import AppShell from './components/AppShell';
import Landing from './pages/Landing';
import DashboardLayout from './pages/dashboard/Layout';
import CostDashboard from './pages/dashboard/CostDashboard';
import AgentFleet from './pages/dashboard/AgentFleet';
import KnowledgeGraph from './pages/dashboard/KnowledgeGraph';
import ChainView from './pages/dashboard/ChainView';
import CascadeRouter from './pages/dashboard/CascadeRouter';
import KnowledgeEntries from './pages/dashboard/KnowledgeEntries';
import ShareView from './pages/dashboard/ShareView';
import Demo from './pages/Demo';
import Terminal from './pages/Terminal';
import Builder from './pages/Builder';
import Explorer from './pages/Explorer';
import Bench from './pages/Bench';
import BenchRunDetail from './pages/BenchRunDetail';
import BenchCompare from './pages/BenchCompare';
import BenchShowroom from './pages/BenchShowroom';
import SharePage from './pages/Share';
import JobMarket from './pages/JobMarket';
import './styles/rosedust.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <BrowserRouter>
      <ErrorBoundary>
        <Routes>
          <Route element={<AppShell />}>
            <Route index element={<Landing />} />
            <Route path="dashboard" element={<DashboardLayout />}>
              <Route index element={<CostDashboard />} />
              <Route path="fleet" element={<AgentFleet />} />
              <Route path="knowledge" element={<KnowledgeGraph />} />
              <Route path="chain" element={<ChainView />} />
              <Route path="entries" element={<KnowledgeEntries />} />
              <Route path="routing" element={<CascadeRouter />} />
              <Route path="share/:token" element={<ShareView />} />
            </Route>
            <Route path="demo" element={<Demo />} />
            <Route path="terminal" element={<Terminal />} />
            <Route path="builder" element={<Builder />} />
            <Route path="explorer" element={<Explorer />} />
            <Route path="bench" element={<Bench />} />
            <Route path="bench/run/:id" element={<BenchRunDetail />} />
            <Route path="bench/compare" element={<BenchCompare />} />
            <Route path="bench/showroom" element={<BenchShowroom />} />
            <Route path="share/:token" element={<SharePage />} />
            <Route path="job-market" element={<JobMarket />} />
          </Route>
        </Routes>
      </ErrorBoundary>
    </BrowserRouter>
  </StrictMode>,
);
