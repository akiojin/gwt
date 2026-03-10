namespace Gwt.Core.Models
{
    /// <summary>
    /// GFM (GitHub Flavored Markdown) コンテンツを保持するモデル。
    /// すべての Markdown 表示で完全な GFM サポートを提供する。
    /// </summary>
    [System.Serializable]
    public class GfmMarkdownContent
    {
        /// <summary>生の Markdown テキスト（GFM 形式）</summary>
        public string RawMarkdown;

        /// <summary>コンテンツの種別（summary, pr_description, issue_body 等）</summary>
        public string ContentType;

        /// <summary>GFM 拡張機能フラグ: テーブル</summary>
        public bool EnableTables = true;

        /// <summary>GFM 拡張機能フラグ: タスクリスト</summary>
        public bool EnableTaskLists = true;

        /// <summary>GFM 拡張機能フラグ: 取り消し線</summary>
        public bool EnableStrikethrough = true;

        /// <summary>GFM 拡張機能フラグ: シンタックスハイライト</summary>
        public bool EnableSyntaxHighlight = true;

        /// <summary>GFM 拡張機能フラグ: オートリンク</summary>
        public bool EnableAutolinks = true;
    }
}
