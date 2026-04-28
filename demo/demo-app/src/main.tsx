import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, Route, Routes } from 'react-router';
import Layout from './components/Layout';
import Bench from './pages/Bench';
import BenchLive from './pages/BenchLive';
import Builder from './pages/Builder';
import Demo from './pages/Demo';
import Explorer from './pages/Explorer';
import Home from './pages/Home';
import Terminal from './pages/Terminal';
import CascadeRouter from './pages/dashboard/CascadeRouter';
import DashboardLayout from './pages/dashboard/Layout';
import KnowledgeEntries from './pages/dashboard/KnowledgeEntries';
import ShareView from './pages/dashboard/ShareView';
import './styles/global.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <BrowserRouter>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<Home />} />
          <Route path="demo" element={<Demo />} />
          <Route path="terminal" element={<Terminal />} />
          <Route path="builder" element={<Builder />} />
          <Route path="explorer" element={<Explorer />} />
          <Route path="bench" element={<Bench />} />
          <Route path="bench-live" element={<BenchLive />} />
          <Route path="dashboard" element={<DashboardLayout />}>
            <Route index element={<KnowledgeEntries />} />
            <Route path="entries" element={<KnowledgeEntries />} />
            <Route path="routing" element={<CascadeRouter />} />
            <Route path="share/:token" element={<ShareView />} />
          </Route>
        </Route>
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
