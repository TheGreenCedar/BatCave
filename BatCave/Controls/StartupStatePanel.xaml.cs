using BatCave.ViewModels;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.ComponentModel;

namespace BatCave.Controls;

public sealed partial class StartupStatePanel : UserControl
{
    private MonitoringShellViewModel? _viewModel;
    private bool _wasStartupErrorVisible;

    public StartupStatePanel()
    {
        InitializeComponent();
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        DataContextChanged += OnDataContextChanged;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        AttachViewModel(DataContext as MonitoringShellViewModel);
        FocusRetryButtonIfStartupErrorAppeared(force: true);
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        AttachViewModel(null);
    }

    private void OnDataContextChanged(FrameworkElement sender, DataContextChangedEventArgs args)
    {
        AttachViewModel(args.NewValue as MonitoringShellViewModel);
        FocusRetryButtonIfStartupErrorAppeared(force: true);
    }

    private void AttachViewModel(MonitoringShellViewModel? viewModel)
    {
        if (ReferenceEquals(_viewModel, viewModel))
        {
            _wasStartupErrorVisible = viewModel?.IsStartupError == true;
            return;
        }

        if (_viewModel is not null)
        {
            _viewModel.PropertyChanged -= OnViewModelPropertyChanged;
        }

        _viewModel = viewModel;

        if (_viewModel is not null)
        {
            _viewModel.PropertyChanged += OnViewModelPropertyChanged;
        }

        _wasStartupErrorVisible = _viewModel?.IsStartupError == true;
    }

    private void OnViewModelPropertyChanged(object? sender, PropertyChangedEventArgs args)
    {
        if (args.PropertyName is nameof(MonitoringShellViewModel.IsStartupError)
            or nameof(MonitoringShellViewModel.StartupErrorVisibility))
        {
            FocusRetryButtonIfStartupErrorAppeared();
        }
    }

    private void FocusRetryButtonIfStartupErrorAppeared(bool force = false)
    {
        bool isStartupErrorVisible = _viewModel?.IsStartupError == true;
        if (!isStartupErrorVisible)
        {
            _wasStartupErrorVisible = false;
            return;
        }

        if (!force && _wasStartupErrorVisible)
        {
            return;
        }

        _wasStartupErrorVisible = true;
        _ = DispatcherQueue.TryEnqueue(() =>
        {
            if (_viewModel?.IsStartupError == true)
            {
                RetryBootstrapButton.Focus(FocusState.Programmatic);
            }
        });
    }
}
