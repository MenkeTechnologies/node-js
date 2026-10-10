for (const src of ['1; do { 2; try { 5; } finally { break; } } while (false)', '1; l: do { 2; try { 5; } finally { break l; } } while (true)',
  '1; do { try { 2; break; } finally { 3; } } while (false)', '1; do { 2; try { break; } finally { 3; } } while (false)',
  '1; for (var i = 0; i < 3; i++) { try { i; } finally { continue; } }', '1; try { 2; } finally { 3; }', 'do { 4; try { 5; } finally { 6; break; } } while (0)'])
  console.log(JSON.stringify(src), (0, eval)(src));
