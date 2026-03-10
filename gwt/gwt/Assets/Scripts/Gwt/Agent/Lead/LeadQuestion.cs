using System.Collections.Generic;

namespace Gwt.Agent.Lead
{
    /// <summary>
    /// Lead からユーザーへの質問モデル。
    /// World Space Canvas 上のフローティング UI として表示される。
    /// "?" マーカー + バルーン（質問テキスト + 選択肢ボタン）で構成。
    /// </summary>
    [System.Serializable]
    public class LeadQuestion
    {
        public string QuestionId;
        public string Text;
        public List<LeadQuestionChoice> Choices = new();
        public string SelectedChoiceId;
        public bool IsAnswered;
    }

    /// <summary>
    /// 質問の選択肢。バルーン上のボタンとして表示される。
    /// ユーザーがボタンをクリックして回答する。
    /// </summary>
    [System.Serializable]
    public class LeadQuestionChoice
    {
        public string Id;
        public string Label;
    }
}
