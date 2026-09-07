using UnityEngine;

namespace Chill.Unity
{
    [DisallowMultipleComponent]
    [AddComponentMenu("Chill/Semantic Event")]
    public sealed class ChillEvent : MonoBehaviour
    {
        [SerializeField]
        private string semanticName = "";

        [SerializeField]
        private ChillEventClass eventClass = ChillEventClass.Domain;

        [SerializeField]
        private ChillEventSeverity severity = ChillEventSeverity.Info;

        public void Observe()
        {
            if (!ChillRuntime.IsConfigured)
            {
                return;
            }
            ChillRuntime.Event(
                semanticName,
                ChillContext.Resolve(transform),
                eventClass,
                severity);
        }
    }
}
