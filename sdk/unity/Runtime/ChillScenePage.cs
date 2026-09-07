using UnityEngine;

namespace Chill.Unity
{
    [DisallowMultipleComponent]
    [AddComponentMenu("Chill/Scene Page")]
    public sealed class ChillScenePage : MonoBehaviour
    {
        [SerializeField]
        private string segment = "";

        [SerializeField]
        private ChillPageRelation relation = ChillPageRelation.Root;

        [SerializeField]
        private ChillPageCause cause = ChillPageCause.Initial;

        [SerializeField]
        private bool inheritCurrentPath;

        private ChillPageScope scope;

        private void OnEnable()
        {
            TryStart();
        }

        private void Start()
        {
            TryStart();
        }

        private void OnDisable()
        {
            if (scope != null)
            {
                scope.Dispose();
                scope = null;
            }
        }

        private void TryStart()
        {
            if (scope != null || !ChillRuntime.IsConfigured || !ChillNames.IsSemantic(segment))
            {
                return;
            }
            scope = ChillRuntime.StartPage(
                segment,
                ChillContext.Resolve(transform),
                relation,
                cause,
                inheritCurrentPath);
        }
    }
}
