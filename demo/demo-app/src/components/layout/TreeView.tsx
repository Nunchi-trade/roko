import {
  useState,
  useCallback,
  useRef,
  useEffect,
  type ReactNode,
  type KeyboardEvent,
} from 'react';
import './TreeView.css';

/* ── public types ── */

export interface TreeNode {
  id: string;
  label: string;
  icon?: ReactNode;
  children?: TreeNode[];
  badge?: ReactNode;
  status?: 'default' | 'active' | 'success' | 'error' | 'warning';
}

export interface TreeViewProps {
  nodes: TreeNode[];
  expanded?: Set<string>;
  onToggle?: (id: string) => void;
  onSelect?: (id: string) => void;
  selected?: string;
  indentSize?: number;
  className?: string;
}

/* ── helpers ── */

/** Flatten tree into an ordered list of visible node ids for keyboard nav. */
function flattenVisible(
  nodes: TreeNode[],
  expandedSet: Set<string>,
): string[] {
  const out: string[] = [];
  const walk = (list: TreeNode[]) => {
    for (const n of list) {
      out.push(n.id);
      if (n.children?.length && expandedSet.has(n.id)) {
        walk(n.children);
      }
    }
  };
  walk(nodes);
  return out;
}

/** Find a node by id in a tree. */
function findNode(nodes: TreeNode[], id: string): TreeNode | undefined {
  for (const n of nodes) {
    if (n.id === id) return n;
    if (n.children) {
      const found = findNode(n.children, id);
      if (found) return found;
    }
  }
  return undefined;
}

/** Find the parent id of a node. */
function findParentId(
  nodes: TreeNode[],
  targetId: string,
  parentId?: string,
): string | undefined {
  for (const n of nodes) {
    if (n.id === targetId) return parentId;
    if (n.children) {
      const found = findParentId(n.children, targetId, n.id);
      if (found !== undefined) return found;
    }
  }
  return undefined;
}

/* ── main TreeView ── */

export default function TreeView({
  nodes,
  expanded: controlledExpanded,
  onToggle: controlledToggle,
  onSelect,
  selected,
  indentSize = 16,
  className,
}: TreeViewProps) {
  // Internal expanded state (uncontrolled mode)
  const [internalExpanded, setInternalExpanded] = useState<Set<string>>(new Set());
  const isControlled = controlledExpanded !== undefined;
  const expandedSet = isControlled ? controlledExpanded : internalExpanded;

  const [focusedId, setFocusedId] = useState<string | null>(null);

  const handleToggle = useCallback(
    (id: string) => {
      if (isControlled) {
        controlledToggle?.(id);
      } else {
        setInternalExpanded((prev) => {
          const next = new Set(prev);
          if (next.has(id)) next.delete(id);
          else next.add(id);
          return next;
        });
      }
    },
    [isControlled, controlledToggle],
  );

  const handleSelect = useCallback(
    (id: string) => {
      onSelect?.(id);
    },
    [onSelect],
  );

  /* keyboard navigation */
  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLDivElement>) => {
      const visible = flattenVisible(nodes, expandedSet);
      if (!visible.length) return;

      const idx = focusedId ? visible.indexOf(focusedId) : -1;

      switch (e.key) {
        case 'ArrowDown': {
          e.preventDefault();
          const next = Math.min(idx + 1, visible.length - 1);
          setFocusedId(visible[next]);
          break;
        }
        case 'ArrowUp': {
          e.preventDefault();
          const prev = Math.max(idx - 1, 0);
          setFocusedId(visible[prev]);
          break;
        }
        case 'ArrowRight': {
          e.preventDefault();
          if (focusedId) {
            const node = findNode(nodes, focusedId);
            if (node?.children?.length && !expandedSet.has(focusedId)) {
              handleToggle(focusedId);
            } else if (node?.children?.length && expandedSet.has(focusedId)) {
              // Move to first child
              setFocusedId(node.children[0].id);
            }
          }
          break;
        }
        case 'ArrowLeft': {
          e.preventDefault();
          if (focusedId) {
            if (expandedSet.has(focusedId)) {
              handleToggle(focusedId);
            } else {
              // Move to parent
              const parentId = findParentId(nodes, focusedId);
              if (parentId) setFocusedId(parentId);
            }
          }
          break;
        }
        case 'Enter':
        case ' ': {
          e.preventDefault();
          if (focusedId) {
            handleSelect(focusedId);
            const node = findNode(nodes, focusedId);
            if (node?.children?.length) handleToggle(focusedId);
          }
          break;
        }
        default:
          break;
      }
    },
    [nodes, expandedSet, focusedId, handleToggle, handleSelect],
  );

  // Initialize focus to first node
  useEffect(() => {
    if (focusedId === null && nodes.length) {
      setFocusedId(nodes[0].id);
    }
  }, [nodes, focusedId]);

  return (
    <div
      className={`tree-view${className ? ` ${className}` : ''}`}
      role="tree"
      tabIndex={0}
      onKeyDown={handleKeyDown}
    >
      {nodes.map((node) => (
        <ConnectedNodeRow
          key={node.id}
          node={node}
          depth={0}
          indentSize={indentSize}
          expandedSet={expandedSet}
          selectedId={selected}
          onToggle={handleToggle}
          onSelect={handleSelect}
          focusedId={focusedId}
          onFocus={setFocusedId}
        />
      ))}
    </div>
  );
}

/* ── connected node: wires expanded/selected from tree state ── */

interface ConnectedNodeRowProps {
  node: TreeNode;
  depth: number;
  indentSize: number;
  expandedSet: Set<string>;
  selectedId?: string;
  onToggle: (id: string) => void;
  onSelect: (id: string) => void;
  focusedId: string | null;
  onFocus: (id: string) => void;
}

function ConnectedNodeRow({
  node,
  depth,
  indentSize,
  expandedSet,
  selectedId,
  onToggle,
  onSelect,
  focusedId,
  onFocus,
}: ConnectedNodeRowProps) {
  const isExpanded = expandedSet.has(node.id);
  const isSelected = selectedId === node.id;
  const hasChildren = !!node.children?.length;
  const childrenRef = useRef<HTMLDivElement>(null);

  // Animate children height
  useEffect(() => {
    const el = childrenRef.current;
    if (!el) return;

    if (isExpanded) {
      el.style.height = '0px';
      void el.offsetHeight;
      el.style.height = `${el.scrollHeight}px`;
      const onEnd = () => {
        el.style.height = 'auto';
      };
      el.addEventListener('transitionend', onEnd, { once: true });
      return () => el.removeEventListener('transitionend', onEnd);
    } else {
      el.style.height = `${el.scrollHeight}px`;
      void el.offsetHeight;
      el.style.height = '0px';
    }
  }, [isExpanded]);

  const rowRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (focusedId === node.id && rowRef.current) {
      rowRef.current.focus({ preventScroll: false });
    }
  }, [focusedId, node.id]);

  const handleClick = () => {
    onSelect(node.id);
    if (hasChildren) onToggle(node.id);
    onFocus(node.id);
  };

  const cls = [
    'tree-node__row',
    isSelected && 'tree-node__row--selected',
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div className="tree-node" role="treeitem" aria-expanded={hasChildren ? isExpanded : undefined}>
      <div
        ref={rowRef}
        className={cls}
        tabIndex={focusedId === node.id ? 0 : -1}
        onClick={handleClick}
        data-node-id={node.id}
      >
        {/* indent guides */}
        <span className="tree-node__indent" style={{ width: depth * indentSize }}>
          {Array.from({ length: depth }, (_, i) => (
            <span
              key={i}
              className="tree-node__guide"
              style={{ height: 28, marginLeft: i * indentSize }}
            />
          ))}
        </span>

        {/* chevron */}
        <span
          className={[
            'tree-node__chevron',
            hasChildren && isExpanded && 'tree-node__chevron--expanded',
            !hasChildren && 'tree-node__chevron--leaf',
          ]
            .filter(Boolean)
            .join(' ')}
        >
          {'\u25B8'}
        </span>

        {/* status dot */}
        {node.status && (
          <span className={`tree-node__status tree-node__status--${node.status}`} />
        )}

        {/* icon */}
        {node.icon && <span className="tree-node__icon">{node.icon}</span>}

        {/* label */}
        <span className="tree-node__label">{node.label}</span>

        {/* badge */}
        {node.badge && <span className="tree-node__badge">{node.badge}</span>}
      </div>

      {/* children */}
      {hasChildren && (
        <div
          ref={childrenRef}
          className="tree-node__children"
          role="group"
          style={{ height: 0, overflow: 'hidden' }}
        >
          {node.children!.map((child) => (
            <ConnectedNodeRow
              key={child.id}
              node={child}
              depth={depth + 1}
              indentSize={indentSize}
              expandedSet={expandedSet}
              selectedId={selectedId}
              onToggle={onToggle}
              onSelect={onSelect}
              focusedId={focusedId}
              onFocus={onFocus}
            />
          ))}
        </div>
      )}
    </div>
  );
}
