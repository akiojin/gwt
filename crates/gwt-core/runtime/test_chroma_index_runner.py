import tempfile
import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import chroma_index_runner as runner


class ChromaIndexRunnerTests(unittest.TestCase):
    def test_classify_file_bucket_separates_code_docs_and_skip(self):
        self.assertEqual(runner.classify_file_bucket('crates/gwt-core/src/runtime.rs'), 'code')
        self.assertEqual(runner.classify_file_bucket('README.md'), 'docs')
        self.assertEqual(runner.classify_file_bucket('docs/search.md'), 'docs')
        self.assertEqual(runner.classify_file_bucket('.claude/skills/gwt-search/SKILL.md'), 'skip')
        self.assertEqual(runner.classify_file_bucket('.codex/skills/gwt-search/SKILL.md'), 'skip')
        self.assertEqual(runner.classify_file_bucket('specs/SPEC-9/spec.md'), 'skip')
        self.assertEqual(runner.classify_file_bucket('specs-archive/SPEC-9/spec.md'), 'skip')
        self.assertEqual(runner.classify_file_bucket('crates/gwt-tui/tests/snapshots/view.snap'), 'skip')

    def test_index_files_writes_code_and_docs_collections_without_embedded_assets(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'project'
            db = Path(tmp) / 'db'
            (root / 'src').mkdir(parents=True)
            (root / 'docs').mkdir(parents=True)
            (root / '.claude' / 'skills' / 'foo').mkdir(parents=True)
            (root / 'specs' / 'SPEC-1').mkdir(parents=True)
            (root / 'crates' / 'gwt-tui' / 'tests' / 'snapshots').mkdir(parents=True)

            (root / 'src' / 'runtime.rs').write_text('//! Runtime bootstrap manager\nfn main() {}\n')
            (root / 'README.md').write_text('# Runtime Overview\n')
            (root / 'docs' / 'search.md').write_text('# Search Guide\n')
            (root / '.claude' / 'skills' / 'foo' / 'SKILL.md').write_text('# Ignored\n')
            (root / 'specs' / 'SPEC-1' / 'spec.md').write_text('# Ignored Spec\n')
            (root / 'crates' / 'gwt-tui' / 'tests' / 'snapshots' / 'view.snap').write_text('ignored snapshot\n')

            result = runner.action_index(str(root), str(db))
            self.assertTrue(result['ok'])
            self.assertEqual(result['codeFilesIndexed'], 1)
            self.assertEqual(result['docFilesIndexed'], 2)
            self.assertEqual(result['filesIndexed'], 3)

            import chromadb

            client = chromadb.PersistentClient(path=str(db))
            code = client.get_collection(runner.CODE_COLLECTION)
            docs = client.get_collection(runner.DOC_COLLECTION)
            self.assertEqual(code.count(), 1)
            self.assertEqual(docs.count(), 2)
            self.assertEqual(code.get()['ids'], ['src/runtime.rs'])
            self.assertCountEqual(docs.get()['ids'], ['README.md', 'docs/search.md'])

    def test_search_files_returns_code_only_and_search_files_docs_returns_docs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'project'
            db = Path(tmp) / 'db'
            (root / 'src').mkdir(parents=True)
            (root / 'docs').mkdir(parents=True)

            (root / 'src' / 'runtime.rs').write_text('//! Runtime bootstrap manager\nfn main() {}\n')
            (root / 'docs' / 'runtime.md').write_text('# Runtime bootstrap guide\n')

            runner.action_index(str(root), str(db))

            code_result = runner.action_search(str(db), 'runtime bootstrap', 5)
            self.assertTrue(code_result['ok'])
            self.assertEqual(code_result['results'][0]['path'], 'src/runtime.rs')
            self.assertNotIn('docs/runtime.md', [item['path'] for item in code_result['results']])

            docs_result = runner.action_search_docs(str(db), 'runtime bootstrap', 5)
            self.assertTrue(docs_result['ok'])
            self.assertEqual(docs_result['results'][0]['path'], 'docs/runtime.md')


if __name__ == '__main__':
    unittest.main()
