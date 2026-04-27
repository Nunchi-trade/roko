import { BrowserRouter, Routes, Route } from 'react-router';
import Layout from './components/Layout';
import Home from './pages/Home';
import Demo from './pages/Demo';
import Terminal from './pages/Terminal';
import Builder from './pages/Builder';
import Explorer from './pages/Explorer';
import Bench from './pages/Bench';
import BenchLive from './pages/BenchLive';

export default function App() {
  return (
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
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
