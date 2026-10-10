interface MdNode {
  type: string;
  lang?: string | null;
  children?: MdNode[];
}

/**
 * The config examples that must be refused are fenced as ```json,invalid so that
 * `crates/core/tests/docs_reference.rs` can hold them to the real parser. Shiki has no
 * such language; highlight them as JSON.
 */
export function remarkJsonInvalid() {
  return (tree: MdNode) => {
    const walk = (node: MdNode) => {
      if (node.type === 'code' && node.lang === 'json,invalid') node.lang = 'json';
      node.children?.forEach(walk);
    };
    walk(tree);
  };
}
